// Copyright (C) 2026 Ryan Daum <ryan.daum@gmail.com> This program is free
// software: you can redistribute it and/or modify it under the terms of the GNU
// Affero General Public License as published by the Free Software Foundation,
// version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more
// details.
//
// You should have received a copy of the GNU Affero General Public License along
// with this program. If not, see <https://www.gnu.org/licenses/>.

//! The shared endpoint: one UDP socket and one task driving every peer's `str0m` engine.
//!
//! Sessions talk to the task through [`EndpointHandle`] commands. The task owns all
//! engine state, so nothing is locked. `str0m` requires that every mutation of an
//! engine (an input, a write, a negotiation) is followed by draining its output until
//! it yields a timeout; [`Peer::drain`] does that after each one.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Instant,
};

use str0m::{
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
    change::SdpOffer,
    channel::ChannelId,
    net::{Protocol, Receive},
};
use tokio::{
    net::UdpSocket,
    sync::{mpsc, oneshot, watch},
};
use tracing::{debug, info, warn};

use crate::host::client_data::WireFormat;
use crate::latest_queue::{LatestQueue, MessageKey};

/// Commands queued for the endpoint. Sends beyond this are dropped, and their events
/// fall back to the WebSocket.
const COMMAND_CAPACITY: usize = 1024;

/// Bytes a channel may have in flight before new messages wait, latest-wins, for it to drain.
const CHANNEL_HIGH_WATER: usize = 64 * 1024;

/// The channel reports it has drained when its buffer falls below this.
const CHANNEL_LOW_WATER: usize = 16 * 1024;

/// Client messages waiting for the session. More are dropped until it catches up.
const INBOUND_CAPACITY: usize = 256;

/// Large enough for any datagram on a typical path MTU.
const RECEIVE_BUFFER: usize = 2048;

pub(super) type PeerId = u64;

/// What a session's peer handle can see of its peer.
pub(super) struct PeerLink {
    /// Whether the client's channel is open; watchers see each change.
    open: watch::Sender<bool>,
    /// The open channel's format, from its sub-protocol.
    format: Mutex<WireFormat>,
}

impl Default for PeerLink {
    fn default() -> Self {
        Self {
            open: watch::Sender::new(false),
            format: Mutex::default(),
        }
    }
}

impl PeerLink {
    pub(super) fn is_open(&self) -> bool {
        *self.open.borrow()
    }

    pub(super) fn watch_open(&self) -> watch::Receiver<bool> {
        self.open.subscribe()
    }

    pub(super) fn format(&self) -> WireFormat {
        *self.format.lock().unwrap()
    }

    fn set_open(&self, open: bool) {
        self.open
            .send_if_modified(|current| std::mem::replace(current, open) != open);
    }

    fn opened(&self, format: WireFormat) {
        *self.format.lock().unwrap() = format;
        self.set_open(true);
    }
}

/// A newly answered peer.
pub(super) struct Answered {
    pub(super) id: PeerId,
    pub(super) sdp: String,
    pub(super) link: Arc<PeerLink>,
    /// Messages the client sends on its channel.
    pub(super) inbound: mpsc::Receiver<Vec<u8>>,
}

pub(super) enum Command {
    Answer {
        offer: SdpOffer,
        reply: oneshot::Sender<Result<Answered, String>>,
    },
    Candidate {
        peer: PeerId,
        candidate: Candidate,
    },
    Send {
        peer: PeerId,
        key: MessageKey,
        data: Vec<u8>,
    },
    Close {
        peer: PeerId,
    },
    #[cfg(test)]
    PeerCount {
        reply: oneshot::Sender<usize>,
    },
}

/// A cheap, cloneable way to reach the endpoint task. The task ends when the last
/// handle is dropped.
#[derive(Clone)]
pub(super) struct EndpointHandle {
    commands: mpsc::Sender<Command>,
    local_addr: SocketAddr,
    advertised: Arc<Vec<SocketAddr>>,
}

impl EndpointHandle {
    /// Bind the socket, advertise `ips` on its port, and start the endpoint task.
    pub(super) async fn bind(listen: SocketAddr, ips: &[IpAddr]) -> eyre::Result<Self> {
        let socket = UdpSocket::bind(listen)
            .await
            .map_err(|e| eyre::eyre!("Unable to bind WebRTC endpoint on udp {listen}: {e}"))?;
        let local_addr = socket.local_addr()?;
        let advertised: Arc<Vec<SocketAddr>> = Arc::new(
            ips.iter()
                .map(|ip| SocketAddr::new(*ip, local_addr.port()))
                .collect(),
        );
        let (commands, queue) = mpsc::channel(COMMAND_CAPACITY);
        let endpoint = Endpoint {
            socket,
            advertised: advertised.clone(),
            peers: HashMap::new(),
            next_id: 0,
        };
        tokio::spawn(endpoint.run(queue));
        Ok(Self {
            commands,
            local_addr,
            advertised,
        })
    }

    pub(super) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub(super) fn advertised(&self) -> &[SocketAddr] {
        &self.advertised
    }

    /// Queue a command, waiting for room. For commands that must not be lost.
    pub(super) async fn request(&self, command: Command) -> eyre::Result<()> {
        self.commands
            .send(command)
            .await
            .map_err(|_| eyre::eyre!("WebRTC endpoint has stopped"))
    }

    /// Queue a command if there is room right now; returns whether it was queued.
    pub(super) fn try_request(&self, command: Command) -> bool {
        self.commands.try_send(command).is_ok()
    }
}

struct Endpoint {
    socket: UdpSocket,
    advertised: Arc<Vec<SocketAddr>>,
    peers: HashMap<PeerId, Peer>,
    next_id: PeerId,
}

impl Endpoint {
    async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        let mut buffer = vec![0; RECEIVE_BUFFER];
        loop {
            let next_timeout = self.peers.values().map(|peer| peer.timeout).min();
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else {
                        break;
                    };
                    self.command(command).await;
                }
                received = self.socket.recv_from(&mut buffer) => match received {
                    Ok((length, source)) => self.receive(source, &buffer[..length]).await,
                    Err(e) => warn!("WebRTC endpoint receive failed: {e}"),
                },
                _ = sleep_until(next_timeout) => self.expire(Instant::now()).await,
            }
            self.reap();
        }
        debug!("WebRTC endpoint stopped");
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::Answer { offer, reply } => {
                let _ = reply.send(self.answer(offer).await);
            }
            Command::Candidate { peer, candidate } => {
                if let Some(peer) = self.peers.get_mut(&peer) {
                    peer.rtc.add_remote_candidate(candidate);
                    peer.drain(&self.socket).await;
                }
            }
            Command::Send { peer, key, data } => {
                if let Some(peer) = self.peers.get_mut(&peer) {
                    peer.pending.push(key, data);
                    peer.flush();
                    peer.drain(&self.socket).await;
                }
            }
            Command::Close { peer } => {
                if let Some(mut peer) = self.peers.remove(&peer) {
                    peer.rtc.disconnect();
                    peer.drain(&self.socket).await;
                    peer.link.set_open(false);
                }
            }
            #[cfg(test)]
            Command::PeerCount { reply } => {
                let _ = reply.send(self.peers.len());
            }
        }
    }

    async fn answer(&mut self, offer: SdpOffer) -> Result<Answered, String> {
        let link = Arc::new(PeerLink::default());
        let (inbound_sender, inbound) = mpsc::channel(INBOUND_CAPACITY);
        let mut peer = Peer {
            rtc: Rtc::builder().set_ice_lite(true).build(Instant::now()),
            link: link.clone(),
            channel: None,
            pending: LatestQueue::default(),
            inbound: inbound_sender,
            timeout: Instant::now(),
        };
        for address in self.advertised.iter() {
            let candidate = Candidate::host(*address, "udp")
                .map_err(|e| format!("Bad WebRTC candidate {address}: {e}"))?;
            peer.rtc.add_local_candidate(candidate);
            peer.drain(&self.socket).await;
        }
        let answer = peer
            .rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|e| format!("Unable to accept WebRTC offer: {e}"))?;
        peer.drain(&self.socket).await;
        let id = self.next_id;
        self.next_id += 1;
        self.peers.insert(id, peer);
        info!("WebRTC peer {id} answered");
        Ok(Answered {
            id,
            sdp: answer.to_sdp_string(),
            link,
            inbound,
        })
    }

    /// Hand a datagram to the peer it belongs to.
    async fn receive(&mut self, source: SocketAddr, datagram: &[u8]) {
        // Peers know the endpoint by its advertised address, which behind NAT is not
        // the socket's own; present each datagram as addressed to it.
        let Some(destination) = self
            .advertised
            .iter()
            .find(|address| address.is_ipv4() == source.is_ipv4())
            .copied()
        else {
            return;
        };
        let Ok(receive) = Receive::new(Protocol::Udp, source, destination, datagram) else {
            return;
        };
        let input = Input::Receive(Instant::now(), receive);
        let Some(peer) = self
            .peers
            .values_mut()
            .find(|peer| peer.rtc.accepts(&input))
        else {
            return;
        };
        if let Err(e) = peer.rtc.handle_input(input) {
            debug!("WebRTC peer rejected a datagram from {source}: {e}");
        }
        peer.drain(&self.socket).await;
    }

    /// Advance every peer whose timer is due.
    async fn expire(&mut self, now: Instant) {
        for peer in self.peers.values_mut().filter(|peer| peer.timeout <= now) {
            if let Err(e) = peer.rtc.handle_input(Input::Timeout(now)) {
                debug!("WebRTC peer timeout failed: {e}");
            }
            peer.drain(&self.socket).await;
        }
    }

    /// Forget peers whose connections have ended.
    fn reap(&mut self) {
        self.peers.retain(|id, peer| {
            let alive = peer.rtc.is_alive();
            if !alive {
                peer.link.set_open(false);
                info!("WebRTC peer {id} disconnected");
            }
            alive
        });
    }
}

struct Peer {
    rtc: Rtc,
    link: Arc<PeerLink>,
    /// The client's channel. Ephemeral events go to the most recently opened one.
    channel: Option<ChannelId>,
    /// Messages waiting for the channel to drain, at most one per key.
    pending: LatestQueue,
    /// Where the client's channel messages go.
    inbound: mpsc::Sender<Vec<u8>>,
    /// When the engine next needs `Input::Timeout`.
    timeout: Instant,
}

impl Peer {
    /// Write waiting messages, oldest first, while the channel has room. Whatever does not
    /// fit stays queued, where a newer message with the same key replaces it.
    fn flush(&mut self) {
        let Some(mut channel) = self.channel.and_then(|id| self.rtc.channel(id)) else {
            return;
        };
        while channel.buffered_amount() < CHANNEL_HIGH_WATER {
            let Some((key, data)) = self.pending.pop() else {
                return;
            };
            match channel.write(true, &data) {
                Ok(true) => {}
                Ok(false) => {
                    self.pending.unpop(key, data);
                    return;
                }
                Err(e) => debug!("WebRTC channel write failed: {e}"),
            }
        }
    }

    /// Transmit and dispatch the engine's output until it next needs a timeout.
    async fn drain(&mut self, socket: &UdpSocket) {
        loop {
            match self.rtc.poll_output() {
                Ok(Output::Timeout(timeout)) => {
                    self.timeout = timeout;
                    return;
                }
                Ok(Output::Transmit(transmit)) => {
                    if let Err(e) = socket
                        .send_to(&transmit.contents, transmit.destination)
                        .await
                    {
                        debug!("WebRTC send to {} failed: {e}", transmit.destination);
                    }
                }
                Ok(Output::Event(event)) => self.event(event),
                Err(e) => {
                    warn!("WebRTC peer failed: {e}");
                    self.rtc.disconnect();
                    self.link.set_open(false);
                    return;
                }
            }
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::ChannelOpen(id, label) => {
                let Some(mut channel) = self.rtc.channel(id) else {
                    return;
                };
                let protocol = channel
                    .config()
                    .map(|config| config.protocol.clone())
                    .unwrap_or_default();
                let Some(format) = WireFormat::for_channel_protocol(&protocol) else {
                    warn!(
                        "WebRTC data channel {label:?} has unknown protocol {protocol:?}; not using it"
                    );
                    return;
                };
                channel.set_buffered_amount_low_threshold(CHANNEL_LOW_WATER);
                info!("WebRTC data channel {label:?} open ({format:?})");
                self.channel = Some(id);
                self.link.opened(format);
                self.flush();
            }
            Event::ChannelData(data) if self.channel == Some(data.id) => {
                if self.inbound.try_send(data.data).is_err() {
                    debug!("WebRTC client message dropped; the session is behind");
                }
            }
            Event::ChannelBufferedAmountLow(id) if self.channel == Some(id) => self.flush(),
            Event::ChannelClose(id) if self.channel == Some(id) => {
                info!("WebRTC data channel closed");
                self.channel = None;
                self.pending.clear();
                self.link.set_open(false);
            }
            Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                self.rtc.disconnect();
                self.link.set_open(false);
            }
            _ => {}
        }
    }
}

/// Sleep until `deadline`, or forever when there is none.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

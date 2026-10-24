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

//! End-to-end tests: `str0m` clients connect to a real endpoint over loopback.

use std::time::{Duration, Instant};

use str0m::{
    Candidate, Event, Input, Output, Rtc,
    change::SdpAnswer,
    channel::{ChannelConfig, Reliability},
    net::{Protocol, Receive},
};
use tokio::{net::UdpSocket, sync::mpsc, sync::oneshot};

use super::endpoint::{Command, EndpointHandle};
use super::*;

const PATIENCE: Duration = Duration::from_secs(10);

async fn webrtc() -> WebRtc {
    WebRtc::start(&WebRtcConfig {
        enabled: true,
        udp_listen: "127.0.0.1:0".parse().unwrap(),
        ..Default::default()
    })
    .await
    .unwrap()
    .unwrap()
}

fn channel(ordered: bool, reliability: Reliability) -> ChannelConfig {
    ChannelConfig {
        label: "realtime".to_string(),
        ordered,
        reliability,
        ..Default::default()
    }
}

/// The browser side: a `str0m` engine on its own loopback socket.
struct Client {
    rtc: Rtc,
    socket: UdpSocket,
    received: mpsc::UnboundedSender<Vec<u8>>,
}

impl Client {
    /// Transmit and dispatch output until the engine next needs a timeout, or `None`
    /// once it has disconnected.
    async fn drain(&mut self) -> Option<Instant> {
        loop {
            match self.rtc.poll_output() {
                Ok(Output::Timeout(timeout)) => return self.rtc.is_alive().then_some(timeout),
                Ok(Output::Transmit(transmit)) => {
                    let _ = self
                        .socket
                        .send_to(&transmit.contents, transmit.destination)
                        .await;
                }
                Ok(Output::Event(Event::ChannelData(data))) => {
                    let _ = self.received.send(data.data);
                }
                Ok(Output::Event(_)) => {}
                Err(_) => return None,
            }
        }
    }

    async fn run(mut self) {
        let local = self.socket.local_addr().unwrap();
        let mut buffer = vec![0; 2048];
        while let Some(timeout) = self.drain().await {
            let wait = timeout.saturating_duration_since(Instant::now());
            let input = tokio::select! {
                received = self.socket.recv_from(&mut buffer) => {
                    let (length, source) = received.unwrap();
                    let receive = Receive::new(Protocol::Udp, source, local, &buffer[..length]).unwrap();
                    Input::Receive(Instant::now(), receive)
                }
                _ = tokio::time::sleep(wait) => Input::Timeout(Instant::now()),
            };
            if self.rtc.handle_input(input).is_err() {
                return;
            }
        }
    }
}

/// Offer `config` from a new client, answer it, and keep the client running. With
/// `trickle`, the client's candidate is signaled after the answer instead of offered.
async fn connect(
    webrtc: &WebRtc,
    config: ChannelConfig,
    trickle: bool,
) -> (WebRtcPeer, mpsc::UnboundedReceiver<Vec<u8>>) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let candidate = Candidate::host(socket.local_addr().unwrap(), "udp").unwrap();
    let (sender, received) = mpsc::unbounded_channel();
    let mut client = Client {
        rtc: Rtc::new(Instant::now()),
        socket,
        received: sender,
    };
    if !trickle {
        client.rtc.add_local_candidate(candidate.clone());
        client.drain().await;
    }
    let mut change = client.rtc.sdp_api();
    change.add_channel_with_config(config);
    let (offer, pending) = change.apply().unwrap();
    client.drain().await;

    let (peer, answer) = webrtc.answer(&offer.to_sdp_string()).await.unwrap();
    client
        .rtc
        .sdp_api()
        .accept_answer(pending, SdpAnswer::from_sdp_string(&answer).unwrap())
        .unwrap();
    client.drain().await;
    if trickle {
        client.rtc.add_local_candidate(candidate.clone());
        client.drain().await;
        peer.add_ice_candidate(&candidate.to_sdp_string())
            .await
            .unwrap();
    }
    tokio::spawn(client.run());
    (peer, received)
}

async fn opened(peer: &WebRtcPeer) {
    tokio::time::timeout(PATIENCE, async {
        while !peer.is_open() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("data channel did not open");
}

async fn peer_count(endpoint: &EndpointHandle) -> usize {
    let (reply, count) = oneshot::channel();
    endpoint
        .request(Command::PeerCount { reply })
        .await
        .unwrap();
    count.await.unwrap()
}

async fn receive(received: &mut mpsc::UnboundedReceiver<Vec<u8>>) -> Vec<u8> {
    tokio::time::timeout(PATIENCE, received.recv())
        .await
        .expect("no message arrived")
        .unwrap()
}

#[tokio::test]
async fn reliable_ordered_channel_delivers_in_order() {
    let webrtc = webrtc().await;
    let (peer, mut received) = connect(&webrtc, channel(true, Reliability::Reliable), false).await;
    opened(&peer).await;
    for sequence in 0..32u8 {
        peer.send(&[sequence]).unwrap();
    }
    for sequence in 0..32u8 {
        assert_eq!(receive(&mut received).await, vec![sequence]);
    }
}

#[tokio::test]
async fn unordered_channel_without_retransmits_delivers() {
    let webrtc = webrtc().await;
    let (peer, mut received) = connect(
        &webrtc,
        channel(false, Reliability::MaxRetransmits { retransmits: 0 }),
        false,
    )
    .await;
    opened(&peer).await;
    peer.send(b"realtime event").unwrap();
    assert_eq!(receive(&mut received).await, b"realtime event");
}

#[tokio::test]
async fn channel_with_packet_lifetime_delivers() {
    let webrtc = webrtc().await;
    let (peer, mut received) = connect(
        &webrtc,
        channel(false, Reliability::MaxPacketLifetime { lifetime: 100 }),
        false,
    )
    .await;
    opened(&peer).await;
    peer.send(b"short lived event").unwrap();
    assert_eq!(receive(&mut received).await, b"short lived event");
}

#[tokio::test]
async fn trickled_client_candidate_connects() {
    let webrtc = webrtc().await;
    let (peer, mut received) = connect(&webrtc, channel(true, Reliability::Reliable), true).await;
    opened(&peer).await;
    peer.send(b"trickled").unwrap();
    assert_eq!(receive(&mut received).await, b"trickled");
}

#[tokio::test]
async fn peers_share_one_endpoint_and_leave_on_drop() {
    let webrtc = webrtc().await;
    let (first, mut first_received) =
        connect(&webrtc, channel(true, Reliability::Reliable), false).await;
    let (second, mut second_received) =
        connect(&webrtc, channel(true, Reliability::Reliable), false).await;
    opened(&first).await;
    opened(&second).await;
    assert_eq!(peer_count(&webrtc.endpoint).await, 2);

    first.send(b"to first").unwrap();
    second.send(b"to second").unwrap();
    assert_eq!(receive(&mut first_received).await, b"to first");
    assert_eq!(receive(&mut second_received).await, b"to second");

    drop(first);
    assert_eq!(peer_count(second.endpoint()).await, 1);
    second.send(b"still here").unwrap();
    assert_eq!(receive(&mut second_received).await, b"still here");
}

#[tokio::test]
async fn sending_before_the_channel_opens_fails() {
    let webrtc = webrtc().await;
    let offer = {
        let mut rtc = Rtc::new(Instant::now());
        let mut change = rtc.sdp_api();
        change.add_channel_with_config(channel(true, Reliability::Reliable));
        change.apply().unwrap().0
    };
    let (peer, _answer) = webrtc.answer(&offer.to_sdp_string()).await.unwrap();
    assert!(!peer.is_open());
    assert!(peer.send(b"too early").is_err());
}

#[tokio::test]
async fn invalid_offers_and_candidates_are_rejected() {
    let webrtc = webrtc().await;
    assert!(webrtc.answer("not sdp").await.is_err());
    let (peer, _received) = connect(&webrtc, channel(true, Reliability::Reliable), false).await;
    assert!(peer.add_ice_candidate("not a candidate").await.is_err());
    peer.add_ice_candidate("").await.unwrap();
}

#[test]
fn advertised_ips_prefer_configuration_then_a_concrete_listen_address() {
    let configured = WebRtcConfig {
        advertised_ips: vec!["203.0.113.5".parse().unwrap()],
        ..Default::default()
    };
    assert_eq!(
        advertised_ips(&configured).unwrap(),
        configured.advertised_ips
    );
    let concrete = WebRtcConfig {
        udp_listen: "192.0.2.10:7882".parse().unwrap(),
        ..Default::default()
    };
    assert_eq!(
        advertised_ips(&concrete).unwrap(),
        vec!["192.0.2.10".parse::<IpAddr>().unwrap()]
    );
}

#[test]
fn only_ephemeral_data_events_use_the_channel() {
    use moor_common::tasks::NarrativeEvent;
    use moor_runtime_api::api::ClientEvent;
    use moor_var::{Obj, Symbol, v_int};
    let player = Obj::mk_id(5);
    let data = |ephemeral| ClientEvent::Narrative {
        player,
        event: NarrativeEvent::data(
            v_int(0),
            Symbol::mk("game"),
            Symbol::mk("positions"),
            v_int(1),
            ephemeral,
        ),
    };
    assert!(is_channel_event(&data(true)));
    assert!(!is_channel_event(&data(false)));
    let narrative = ClientEvent::Narrative {
        player,
        event: NarrativeEvent::notify(v_int(0), v_int(1), None, false, false, None),
    };
    assert!(!is_channel_event(&narrative));
}

#[tokio::test]
async fn disabled_webrtc_starts_nothing() {
    assert!(
        WebRtc::start(&WebRtcConfig::default())
            .await
            .unwrap()
            .is_none()
    );
}

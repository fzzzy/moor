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

//! A session's handle on its peer in the shared endpoint.

use std::sync::Arc;

use str0m::{Candidate, change::SdpOffer};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use super::endpoint::{Answered, Command, EndpointHandle, MessageKey, PeerId, PeerLink};
use crate::host::client_data::WireFormat;

/// One client's WebRTC connection. Dropping it removes the peer from the endpoint.
pub(crate) struct WebRtcPeer {
    id: PeerId,
    link: Arc<PeerLink>,
    endpoint: EndpointHandle,
}

/// Messages a client sends on its data channel, in the channel's format.
pub(crate) type ChannelMessages = mpsc::Receiver<Vec<u8>>;

impl WebRtcPeer {
    /// Answer a client's offer; returns the peer, the SDP answer, and the client's
    /// channel messages.
    pub(super) async fn answer(
        endpoint: EndpointHandle,
        offer_sdp: &str,
    ) -> eyre::Result<(Self, String, ChannelMessages)> {
        let offer = SdpOffer::from_sdp_string(offer_sdp)
            .map_err(|e| eyre::eyre!("Invalid WebRTC offer: {e}"))?;
        let (reply, answered) = oneshot::channel();
        endpoint.request(Command::Answer { offer, reply }).await?;
        let Answered {
            id,
            sdp,
            link,
            inbound,
        } = answered
            .await
            .map_err(|_| eyre::eyre!("WebRTC endpoint has stopped"))?
            .map_err(|e| eyre::eyre!(e))?;
        Ok((Self { id, link, endpoint }, sdp, inbound))
    }

    /// Add a candidate the client trickled. An empty candidate ends the client's list.
    pub(crate) async fn add_ice_candidate(&self, candidate: &str) -> eyre::Result<()> {
        if candidate.is_empty() {
            return Ok(());
        }
        let candidate = Candidate::from_sdp_string(candidate)
            .map_err(|e| eyre::eyre!("Invalid ICE candidate {candidate:?}: {e}"))?;
        self.endpoint
            .request(Command::Candidate {
                peer: self.id,
                candidate,
            })
            .await
    }

    /// Whether the client's data channel is open.
    pub(crate) fn is_open(&self) -> bool {
        self.link.is_open()
    }

    /// Watch whether the client's data channel is open.
    pub(crate) fn watch_open(&self) -> tokio::sync::watch::Receiver<bool> {
        self.link.watch_open()
    }

    /// The open channel's format, which applies to messages in both directions.
    pub(crate) fn format(&self) -> WireFormat {
        self.link.format()
    }

    /// Queue a binary message for the data channel, superseding any undelivered message
    /// with the same key. Fails when the channel is closed or the endpoint is backlogged,
    /// so the caller can use the WebSocket instead.
    pub(crate) fn send(&self, key: MessageKey, data: &[u8]) -> eyre::Result<()> {
        if !self.is_open() {
            return Err(eyre::eyre!("Data channel not open"));
        }
        let queued = self.endpoint.try_request(Command::Send {
            peer: self.id,
            key,
            data: data.to_vec(),
        });
        queued
            .then_some(())
            .ok_or_else(|| eyre::eyre!("WebRTC endpoint backlogged"))
    }

    #[cfg(test)]
    pub(super) fn endpoint(&self) -> &EndpointHandle {
        &self.endpoint
    }
}

impl Drop for WebRtcPeer {
    fn drop(&mut self) {
        let peer = self.id;
        if self.endpoint.try_request(Command::Close { peer }) {
            return;
        }
        // The queue is full; wait for room rather than leave the peer to time out.
        let endpoint = self.endpoint.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    let _ = endpoint.request(Command::Close { peer }).await;
                });
            }
            Err(_) => warn!("WebRTC peer dropped outside a runtime; it will time out"),
        }
    }
}

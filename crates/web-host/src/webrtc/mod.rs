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

//! WebRTC data channels for realtime event delivery.
//!
//! A client may open a WebRTC data channel alongside its WebSocket. Events whose
//! namespace is in the configured realtime domains then travel over that channel,
//! which the client creates with the ordering and reliability it wants (typically
//! unordered and unreliable, for the lowest latency). Everything else, and any
//! realtime event the channel cannot take, stays on the WebSocket.
//!
//! The host runs one [`str0m`] endpoint: a single UDP socket and task serving every
//! peer, so a firewall or container needs only that one port open. The server is
//! ICE-lite and advertises its own address, so it needs no STUN server and sends no
//! trickled candidates; they are all in its SDP answer. Signaling rides the WebSocket
//! (see [`signaling`]).

mod endpoint;
mod peer;
pub(crate) mod signaling;

#[cfg(test)]
mod tests;

use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
};

use moor_common::tasks::Event as NarrativeEventKind;
use moor_runtime_api::api::ClientEvent;
use serde_derive::{Deserialize, Serialize};
use tracing::info;

use self::endpoint::EndpointHandle;
pub(crate) use self::peer::WebRtcPeer;

/// Configuration for WebRTC data channel support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebRtcConfig {
    #[serde(default)]
    pub enabled: bool,

    /// UDP address of the shared endpoint. Every peer uses this one socket.
    #[serde(default = "WebRtcConfig::default_udp_listen")]
    pub udp_listen: SocketAddr,

    /// IP addresses clients can reach the endpoint on, advertised as ICE candidates
    /// with the endpoint's port. Needed behind NAT, or in a container, where the
    /// listening address is not the one clients see. When empty, a concrete
    /// `udp_listen` address is advertised as is; for a wildcard, the host's address
    /// on its default route is.
    #[serde(default)]
    pub advertised_ips: Vec<IpAddr>,

    /// Event namespaces delivered over the data channel when it is open.
    #[serde(default)]
    pub realtime_domains: Vec<String>,
}

impl WebRtcConfig {
    fn default_udp_listen() -> SocketAddr {
        SocketAddr::from(([0, 0, 0, 0], 7882))
    }
}

impl Default for WebRtcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            udp_listen: Self::default_udp_listen(),
            advertised_ips: vec![],
            realtime_domains: vec![],
        }
    }
}

/// The host's WebRTC service: the shared endpoint and the realtime routing policy.
pub struct WebRtc {
    endpoint: EndpointHandle,
    realtime_domains: HashSet<String>,
}

impl WebRtc {
    /// Bind the endpoint and start serving it, or return `None` when WebRTC is disabled.
    pub async fn start(config: &WebRtcConfig) -> eyre::Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }
        let ips = advertised_ips(config)?;
        let endpoint = EndpointHandle::bind(config.udp_listen, &ips).await?;
        info!(
            "WebRTC endpoint on udp {}, advertising {:?}",
            endpoint.local_addr(),
            endpoint.advertised()
        );
        Ok(Some(Self {
            endpoint,
            realtime_domains: config.realtime_domains.iter().cloned().collect(),
        }))
    }

    /// Answer a client's SDP offer with a new peer on the shared endpoint.
    pub(crate) async fn answer(&self, offer_sdp: &str) -> eyre::Result<(WebRtcPeer, String)> {
        WebRtcPeer::answer(self.endpoint.clone(), offer_sdp).await
    }

    /// Whether an event belongs on the data channel: a data event in a realtime domain.
    pub(crate) fn is_realtime(&self, event: &ClientEvent) -> bool {
        let ClientEvent::Narrative { event, .. } = event else {
            return false;
        };
        let NarrativeEventKind::Data { namespace, .. } = &event.event else {
            return false;
        };
        self.realtime_domains.contains(&namespace.as_string())
    }
}

/// The IPs to advertise, before the endpoint's port is known.
fn advertised_ips(config: &WebRtcConfig) -> eyre::Result<Vec<IpAddr>> {
    if !config.advertised_ips.is_empty() {
        return Ok(config.advertised_ips.clone());
    }
    let listen = config.udp_listen.ip();
    if !listen.is_unspecified() {
        return Ok(vec![listen]);
    }
    let routed = default_route_ip(listen).ok_or_else(|| {
        eyre::eyre!(
            "WebRTC udp_listen {} is a wildcard and the host has no default route; set webrtc.advertised_ips",
            config.udp_listen
        )
    })?;
    Ok(vec![routed])
}

/// The local address the host would send from to reach the internet, if it has a route.
/// Connecting a UDP socket only selects a route; it sends nothing.
fn default_route_ip(family: IpAddr) -> Option<IpAddr> {
    let (bind, target): (SocketAddr, SocketAddr) = match family {
        IpAddr::V4(_) => (([0, 0, 0, 0], 0).into(), ([192, 0, 2, 1], 9).into()),
        IpAddr::V6(_) => (
            (std::net::Ipv6Addr::UNSPECIFIED, 0).into(),
            ("2001:db8::1".parse::<std::net::Ipv6Addr>().ok()?, 9).into(),
        ),
    };
    let socket = std::net::UdpSocket::bind(bind).ok()?;
    socket.connect(target).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

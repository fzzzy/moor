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

//! WebRTC signaling over the WebSocket.
//!
//! A signaling frame is a binary WebSocket message: the byte `0x03`, then JSON. The
//! client sends an `offer` and may trickle `ice` candidates; the server replies with
//! an `answer` that already lists its own candidates.

use serde_derive::{Deserialize, Serialize};

/// WebSocket message prefix for WebRTC signaling messages.
pub(crate) const SIGNALING_PREFIX: u8 = 0x03;

/// JSON envelope for signaling messages exchanged over WebSocket.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum SignalingMessage {
    Offer {
        sdp: String,
    },
    Answer {
        sdp: String,
    },
    /// A client ICE candidate, as the candidate attribute string from the browser's
    /// `RTCIceCandidate.candidate`. An empty string marks the end of candidates.
    #[serde(rename = "ice")]
    IceCandidate {
        candidate: String,
    },
}

/// Parse a signaling message from a WebSocket binary frame.
pub(crate) fn parse_signaling_message(data: &[u8]) -> Option<SignalingMessage> {
    let (&SIGNALING_PREFIX, json) = data.split_first()? else {
        return None;
    };
    serde_json::from_slice(json).ok()
}

/// Encode a signaling message as a WebSocket binary frame.
pub(crate) fn encode_signaling_message(msg: &SignalingMessage) -> Vec<u8> {
    let json = serde_json::to_vec(msg).expect("signaling message serialization");
    let mut frame = Vec::with_capacity(1 + json.len());
    frame.push(SIGNALING_PREFIX);
    frame.extend_from_slice(&json);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let message = SignalingMessage::Answer {
            sdp: "v=0".to_string(),
        };
        let frame = encode_signaling_message(&message);
        assert_eq!(frame[0], SIGNALING_PREFIX);
        assert_eq!(parse_signaling_message(&frame), Some(message));
    }

    #[test]
    fn browser_candidates_parse_and_extra_fields_are_ignored() {
        let frame = [
            &[SIGNALING_PREFIX][..],
            br#"{"type":"ice","candidate":"candidate:1 1 udp 1 192.0.2.7 9 typ host","sdpMid":"0","sdpMLineIndex":0}"#,
        ]
        .concat();
        assert_eq!(
            parse_signaling_message(&frame),
            Some(SignalingMessage::IceCandidate {
                candidate: "candidate:1 1 udp 1 192.0.2.7 9 typ host".to_string()
            })
        );
    }

    #[test]
    fn other_frames_are_not_signaling() {
        assert_eq!(parse_signaling_message(b""), None);
        assert_eq!(parse_signaling_message(b"\x01{}"), None);
        assert_eq!(parse_signaling_message(b"\x03not json"), None);
    }
}

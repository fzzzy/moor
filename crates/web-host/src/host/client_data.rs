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

//! Structured data between web clients and `do_client_data`, in JSON or FlatBuffers.
//!
//! A client sends a `kind` and a `payload`, either as a WebSocket frame (the byte
//! `0x05`, then the message) or on its WebRTC data channel. They reach
//! `handler:do_client_data(connection, 'web, kind, payload)`. The namespace is fixed:
//! clients must not create symbols, so `kind` stays a string.
//!
//! JSON messages are objects, `{"kind": "...", "payload": ...}`; FlatBuffers messages
//! are a `ClientDataMessage` root. A WebSocket frame is JSON when its body starts with
//! `{`. A data channel's format is chosen once, by its sub-protocol (see
//! [`WireFormat::for_channel_protocol`]), and applies in both directions: on a JSON
//! channel the server sends ephemeral events as `{"namespace", "kind", "payload"}`.

use moor_common::tasks::{Event, NarrativeEvent};
use moor_runtime_api::{AuthToken, ClientToken, api::ClientRequest};
use moor_schema::{common as moor_common_schema, convert::var_from_flatbuffer_ref};
use moor_var::{
    Obj, Symbol, Var,
    json::{json_to_var, var_to_json},
};
use planus::ReadAsRoot;
use serde_json::json;

/// Binary WebSocket frame: this byte followed by a client data message.
pub(crate) const CLIENT_DATA_PREFIX: u8 = 0x05;

/// The namespace `do_client_data` receives for web client data.
const WEB_NAMESPACE: &str = "web";

const MAX_CLIENT_DATA_BYTES: usize = 16 * 1024;

/// How client data is encoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum WireFormat {
    #[default]
    FlatBuffers,
    Json,
}

impl WireFormat {
    /// A data channel's format from its sub-protocol: `moor.json`, or `moor.fb` (also
    /// the default for an empty protocol). Other protocols are not ours.
    pub(crate) fn for_channel_protocol(protocol: &str) -> Option<Self> {
        match protocol {
            "" | "moor.fb" => Some(Self::FlatBuffers),
            "moor.json" => Some(Self::Json),
            _ => None,
        }
    }

    /// The format of a WebSocket frame body: JSON objects start with `{`.
    pub(crate) fn of_frame_body(body: &[u8]) -> Self {
        if body.first() == Some(&b'{') {
            Self::Json
        } else {
            Self::FlatBuffers
        }
    }
}

/// Decode a client data message into its kind and payload.
pub(crate) fn decode(format: WireFormat, body: &[u8]) -> Result<(String, Var), &'static str> {
    if body.len() > MAX_CLIENT_DATA_BYTES {
        return Err("Client data is too large");
    }
    match format {
        WireFormat::Json => {
            let value: serde_json::Value =
                serde_json::from_slice(body).map_err(|_| "Invalid client data JSON")?;
            let kind = value
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .ok_or("Client data needs a string kind")?;
            let payload = value.get("payload").unwrap_or(&serde_json::Value::Null);
            Ok((kind.to_string(), json_to_var(payload, false)))
        }
        WireFormat::FlatBuffers => {
            let message = moor_common_schema::ClientDataMessageRef::read_as_root(body)
                .map_err(|_| "Invalid client data message")?;
            let kind = message.kind().map_err(|_| "Invalid client data kind")?;
            let payload = message
                .payload()
                .map_err(|_| "Invalid client data payload")?;
            let payload =
                var_from_flatbuffer_ref(payload).map_err(|_| "Invalid client data payload")?;
            Ok((kind.to_string(), payload))
        }
    }
}

/// The daemon request delivering client data to `handler:do_client_data`.
pub(crate) fn request(
    kind: String,
    payload: Var,
    handler_object: Obj,
    auth_token: AuthToken,
    client_token: ClientToken,
) -> ClientRequest {
    ClientRequest::ClientData {
        client_token,
        auth_token: Some(auth_token),
        handler_object,
        namespace: Symbol::mk(WEB_NAMESPACE),
        kind,
        payload,
    }
}

/// An ephemeral data event as a JSON channel message, or `None` for other events or a
/// payload JSON cannot represent.
pub(crate) fn event_json(event: &NarrativeEvent) -> Option<Vec<u8>> {
    let Event::Data {
        namespace,
        kind,
        payload,
        ..
    } = &event.event
    else {
        return None;
    };
    let message = json!({
        "namespace": namespace.as_string(),
        "kind": kind.as_string(),
        "payload": var_to_json(payload).ok()?,
    });
    serde_json::to_vec(&message).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moor_schema::convert::var_to_flatbuffer;
    use moor_var::{v_int, v_list, v_str};

    fn flatbuffer(kind: &str, payload: &Var) -> Vec<u8> {
        let message = moor_common_schema::ClientDataMessage {
            kind: kind.to_string(),
            payload: Box::new(var_to_flatbuffer(payload).unwrap()),
        };
        let mut builder = planus::Builder::new();
        builder.finish(&message, None).to_vec()
    }

    #[test]
    fn json_and_flatbuffer_messages_decode_alike() {
        let payload = v_list(&[v_int(3), v_int(4)]);
        let json_body = br#"{"kind":"move","payload":[3,4]}"#;
        assert_eq!(WireFormat::of_frame_body(json_body), WireFormat::Json);
        assert_eq!(
            decode(WireFormat::Json, json_body),
            Ok(("move".to_string(), payload.clone()))
        );
        let fb_body = flatbuffer("move", &payload);
        assert_eq!(WireFormat::of_frame_body(&fb_body), WireFormat::FlatBuffers);
        assert_eq!(
            decode(WireFormat::FlatBuffers, &fb_body),
            Ok(("move".to_string(), payload))
        );
    }

    #[test]
    fn malformed_and_oversized_messages_are_rejected() {
        for body in [
            br#"{"payload":1}"#.as_slice(),
            br#"{"kind":7}"#,
            b"[]",
            b"{",
        ] {
            assert!(decode(WireFormat::Json, body).is_err(), "{body:?}");
        }
        assert!(decode(WireFormat::FlatBuffers, b"\x01\x02").is_err());
        let large = format!(
            r#"{{"kind":"k","payload":"{}"}}"#,
            "x".repeat(MAX_CLIENT_DATA_BYTES)
        );
        assert!(decode(WireFormat::Json, large.as_bytes()).is_err());
        // A missing payload is null.
        assert!(decode(WireFormat::Json, br#"{"kind":"ping"}"#).is_ok());
    }

    #[test]
    fn channel_protocols_choose_the_format() {
        assert_eq!(
            WireFormat::for_channel_protocol(""),
            Some(WireFormat::FlatBuffers)
        );
        assert_eq!(
            WireFormat::for_channel_protocol("moor.fb"),
            Some(WireFormat::FlatBuffers)
        );
        assert_eq!(
            WireFormat::for_channel_protocol("moor.json"),
            Some(WireFormat::Json)
        );
        assert_eq!(WireFormat::for_channel_protocol("chat"), None);
    }

    #[test]
    fn requests_use_the_web_namespace_and_keep_the_kind_a_string() {
        let ClientRequest::ClientData {
            namespace, kind, ..
        } = request(
            "move".to_string(),
            v_int(1),
            Obj::mk_id(0),
            AuthToken("a".into()),
            ClientToken("c".into()),
        )
        else {
            panic!("expected client data");
        };
        assert_eq!((namespace, kind.as_str()), (Symbol::mk("web"), "move"));
    }

    #[test]
    fn ephemeral_events_encode_as_json_objects() {
        let event = NarrativeEvent::data(
            v_int(0),
            Symbol::mk("game"),
            Symbol::mk("positions"),
            v_list(&[v_str("lp"), v_int(3)]),
            true,
        );
        let message: serde_json::Value =
            serde_json::from_slice(&event_json(&event).unwrap()).unwrap();
        assert_eq!(
            message,
            json!({"namespace": "game", "kind": "positions", "payload": ["lp", 3]})
        );
        let notify = NarrativeEvent::notify(v_int(0), v_str("hi"), None, false, false, None);
        assert_eq!(event_json(&notify), None);
    }
}

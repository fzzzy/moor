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

use std::fmt;

use crate::RpcMessageError;

/// The longest listener kind name.
pub const MAX_LISTENER_KIND_LEN: usize = 32;

/// The kind of a listener, such as `"telnet"` or `"http"`. A host declares the kinds it can open,
/// and `listen()` names one.
///
/// Kinds are open: any host may name its own. A name is 1 to [`MAX_LISTENER_KIND_LEN`] ASCII
/// lowercase letters, digits, `-` or `_`. It is compared as text and never interned as a symbol.
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct ListenerKind {
    len: u8,
    name: [u8; MAX_LISTENER_KIND_LEN],
}

impl ListenerKind {
    /// The telnet host's line-oriented TCP listener.
    pub const TELNET: ListenerKind = ListenerKind::from_static("telnet");
    /// The web host's HTTP listener, which also serves WebSocket and SSE.
    pub const HTTP: ListenerKind = ListenerKind::from_static("http");

    /// Build a host type from a name known at compile time; panics if the name is invalid.
    #[must_use]
    pub const fn from_static(name: &'static str) -> Self {
        match Self::validate(name.as_bytes()) {
            Ok(kind) => kind,
            Err(_) => panic!("invalid listener kind name"),
        }
    }

    /// Parse a listener kind name.
    pub fn parse(name: &str) -> Result<Self, InvalidListenerKind> {
        Self::validate(name.as_bytes())
    }

    const fn validate(bytes: &[u8]) -> Result<Self, InvalidListenerKind> {
        if bytes.is_empty() || bytes.len() > MAX_LISTENER_KIND_LEN {
            return Err(InvalidListenerKind);
        }
        let mut name = [0u8; MAX_LISTENER_KIND_LEN];
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
                return Err(InvalidListenerKind);
            }
            name[i] = b;
            i += 1;
        }
        Ok(Self {
            len: bytes.len() as u8,
            name,
        })
    }

    #[must_use]
    pub fn id_str(&self) -> &str {
        // Validation admits only ASCII.
        std::str::from_utf8(&self.name[..self.len as usize]).unwrap_or_default()
    }
}

impl fmt::Debug for ListenerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ListenerKind({:?})", self.id_str())
    }
}

impl fmt::Display for ListenerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id_str())
    }
}

/// A listener kind name that is empty, too long, or uses characters outside `[a-z0-9_-]`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct InvalidListenerKind;

impl fmt::Display for InvalidListenerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "listener kind must be 1 to {MAX_LISTENER_KIND_LEN} characters of a-z, 0-9, '-' or '_'"
        )
    }
}

impl std::error::Error for InvalidListenerKind {}

/// Extract and validate a listener kind from a FlatBuffer message.
pub fn extract_listener_kind<T>(
    msg: &T,
    field_name: &str,
    get_field: impl FnOnce(&T) -> Result<&str, planus::Error>,
) -> Result<ListenerKind, RpcMessageError> {
    let name = get_field(msg)
        .map_err(|e| RpcMessageError::InvalidRequest(format!("Missing {field_name}: {e}")))?;
    ListenerKind::parse(name)
        .map_err(|e| RpcMessageError::InvalidRequest(format!("Invalid {field_name}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_round_trip() {
        assert_eq!(ListenerKind::TELNET.id_str(), "telnet");
        assert_eq!(ListenerKind::HTTP.id_str(), "http");
        assert_eq!(
            ListenerKind::parse("minecraft").unwrap().id_str(),
            "minecraft"
        );
        assert_eq!(ListenerKind::parse("telnet").unwrap(), ListenerKind::TELNET);
    }

    #[test]
    fn names_are_validated() {
        assert!(ListenerKind::parse("").is_err());
        assert!(ListenerKind::parse("TCP").is_err());
        assert!(ListenerKind::parse("web host").is_err());
        assert!(ListenerKind::parse(&"a".repeat(MAX_LISTENER_KIND_LEN)).is_ok());
        assert!(ListenerKind::parse(&"a".repeat(MAX_LISTENER_KIND_LEN + 1)).is_err());
        assert!(ListenerKind::parse("my_host-2").is_ok());
    }
}

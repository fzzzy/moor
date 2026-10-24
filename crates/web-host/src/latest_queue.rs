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

//! Latest-wins queues for ephemeral messages, shared by the data channel and the
//! WebSocket writer.

use std::collections::VecDeque;

use moor_var::Symbol;

/// What a message supersedes: an older undelivered one with the same namespace and kind.
pub(crate) type MessageKey = (Symbol, Symbol);

/// Messages waiting to be written, in arrival order, with at most one per key: pushing a
/// message whose key is already waiting replaces that message's data in place.
#[derive(Default)]
pub(crate) struct LatestQueue {
    messages: VecDeque<(MessageKey, Vec<u8>)>,
}

impl LatestQueue {
    pub(crate) fn push(&mut self, key: MessageKey, data: Vec<u8>) {
        match self
            .messages
            .iter_mut()
            .find(|(waiting, _)| *waiting == key)
        {
            Some((_, waiting)) => *waiting = data,
            None => self.messages.push_back((key, data)),
        }
    }

    pub(crate) fn pop(&mut self) -> Option<(MessageKey, Vec<u8>)> {
        self.messages.pop_front()
    }

    /// Put back a message that could not be written, ahead of the rest.
    pub(crate) fn unpop(&mut self, key: MessageKey, data: Vec<u8>) {
        self.messages.push_front((key, data));
    }

    pub(crate) fn clear(&mut self) {
        self.messages.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.messages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_queue_replaces_in_place_and_puts_back_first() {
        let positions = (Symbol::mk("game"), Symbol::mk("positions"));
        let score = (Symbol::mk("game"), Symbol::mk("score"));
        let mut queue = LatestQueue::default();
        queue.push(positions, b"p1".to_vec());
        queue.push(score, b"s1".to_vec());
        queue.push(positions, b"p2".to_vec());
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop(), Some((positions, b"p2".to_vec())));
        queue.unpop(positions, b"p2".to_vec());
        assert_eq!(queue.pop(), Some((positions, b"p2".to_vec())));
        assert_eq!(queue.pop(), Some((score, b"s1".to_vec())));
        assert_eq!(queue.pop(), None);
    }
}

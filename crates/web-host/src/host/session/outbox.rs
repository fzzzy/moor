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

//! The session's WebSocket writer.
//!
//! A task of its own writes to the socket, so a client that reads slowly delays its own
//! output, not the session. Durable messages are written in order and, when the queue is
//! full, make the session wait. Ephemeral events wait in a latest-wins queue instead: a
//! backlog holds at most one per namespace and kind, and a newer event replaces an older
//! one still waiting. The two kinds are not ordered with respect to each other.

use std::sync::{Arc, Mutex};

use axum::extract::ws::Message;
use futures_util::{Sink, SinkExt};
use tokio::{
    sync::{Notify, mpsc},
    task::JoinHandle,
};
use tracing::error;

use crate::latest_queue::{LatestQueue, MessageKey};

/// Durable messages waiting for the writer before the session waits too.
const DURABLE_CAPACITY: usize = 64;

/// Messages for the client's WebSocket, written by their own task.
pub(crate) struct Outbox {
    durable: mpsc::Sender<Message>,
    ephemeral: Arc<Mutex<LatestQueue>>,
    ephemeral_waiting: Arc<Notify>,
    writer: JoinHandle<()>,
}

impl Outbox {
    pub(crate) fn start<W>(socket: W) -> Self
    where
        W: Sink<Message, Error = axum::Error> + Unpin + Send + 'static,
    {
        let (durable, durable_rx) = mpsc::channel(DURABLE_CAPACITY);
        let ephemeral = Arc::new(Mutex::new(LatestQueue::default()));
        let ephemeral_waiting = Arc::new(Notify::new());
        let writer = tokio::spawn(write(
            socket,
            durable_rx,
            ephemeral.clone(),
            ephemeral_waiting.clone(),
        ));
        Self {
            durable,
            ephemeral,
            ephemeral_waiting,
            writer,
        }
    }

    /// Queue a message, waiting while the queue is full. False once the socket has failed.
    pub(crate) async fn send(&self, message: Message) -> bool {
        self.durable.send(message).await.is_ok()
    }

    /// Queue an ephemeral event, replacing a waiting one with the same key. False once the
    /// socket has failed.
    pub(crate) fn send_latest(&self, key: MessageKey, data: Vec<u8>) -> bool {
        if self.durable.is_closed() {
            return false;
        }
        self.ephemeral.lock().unwrap().push(key, data);
        self.ephemeral_waiting.notify_one();
        true
    }
}

impl Drop for Outbox {
    fn drop(&mut self) {
        self.writer.abort();
    }
}

async fn write<W>(
    mut socket: W,
    mut durable: mpsc::Receiver<Message>,
    ephemeral: Arc<Mutex<LatestQueue>>,
    ephemeral_waiting: Arc<Notify>,
) where
    W: Sink<Message, Error = axum::Error> + Unpin,
{
    loop {
        let message = tokio::select! {
            message = durable.recv() => match message {
                Some(message) => message,
                None => return,
            },
            () = ephemeral_waiting.notified() => {
                let mut queue = ephemeral.lock().unwrap();
                let Some((_, data)) = queue.pop() else {
                    continue;
                };
                // One wake-up can stand for several pushes; keep draining.
                if queue.len() > 0 {
                    ephemeral_waiting.notify_one();
                }
                Message::Binary(data.into())
            }
        };
        if let Err(e) = socket.send(message).await {
            error!("Failed to send message to websocket: {e}");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{pin::Pin, time::Duration};

    use moor_var::Symbol;
    use tokio::sync::oneshot;

    use super::*;

    /// A socket that holds its first write until released, then records every write.
    fn socket(
        release: oneshot::Receiver<()>,
        written: mpsc::UnboundedSender<Message>,
    ) -> Pin<Box<dyn Sink<Message, Error = axum::Error> + Send>> {
        Box::pin(futures_util::sink::unfold(
            (Some(release), written),
            |(release, written), message| async move {
                if let Some(release) = release {
                    let _ = release.await;
                }
                written.send(message).map_err(axum::Error::new)?;
                Ok((None, written))
            },
        ))
    }

    async fn next(written: &mut mpsc::UnboundedReceiver<Message>) -> Vec<u8> {
        let message = tokio::time::timeout(Duration::from_secs(10), written.recv())
            .await
            .expect("nothing was written")
            .unwrap();
        message.into_data().to_vec()
    }

    #[tokio::test]
    async fn a_blocked_socket_keeps_only_the_latest_ephemeral_event_per_key() {
        let (release, released) = oneshot::channel();
        let (written_tx, mut written) = mpsc::unbounded_channel();
        let outbox = Outbox::start(socket(released, written_tx));
        let position = (Symbol::mk("game"), Symbol::mk("position"));
        let score = (Symbol::mk("game"), Symbol::mk("score"));

        // The first message occupies the writer until the socket is released.
        assert!(
            outbox
                .send(Message::Binary(b"durable".to_vec().into()))
                .await
        );
        tokio::task::yield_now().await;
        for n in 0..100u8 {
            assert!(outbox.send_latest(position, vec![n]));
        }
        assert!(outbox.send_latest(score, b"score".to_vec()));
        release.send(()).unwrap();

        assert_eq!(next(&mut written).await, b"durable");
        assert_eq!(next(&mut written).await, [99]);
        assert_eq!(next(&mut written).await, b"score");
        tokio::task::yield_now().await;
        assert!(
            written.try_recv().is_err(),
            "a superseded event was written"
        );
    }

    #[tokio::test]
    async fn a_failed_socket_stops_accepting_messages() {
        let (release, released) = oneshot::channel();
        let (written_tx, written) = mpsc::unbounded_channel();
        drop(written);
        let outbox = Outbox::start(socket(released, written_tx));
        release.send(()).unwrap();
        assert!(outbox.send(Message::Binary(b"lost".to_vec().into())).await);
        tokio::time::timeout(Duration::from_secs(10), async {
            while outbox.send(Message::Binary(b"more".to_vec().into())).await {}
        })
        .await
        .expect("the outbox kept accepting messages");
        let key = (Symbol::mk("game"), Symbol::mk("position"));
        assert!(!outbox.send_latest(key, vec![1]));
    }
}

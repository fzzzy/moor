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

//! Dynamic listener lifecycle and per-listener HTTP server tasks.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use axum::Router;
use moor_runtime_api::ListenerKind;
use moor_var::Obj;
use moor_zmq_client::{ListenerInfo, ListenersClient, ListenersError, ListenersMessage};
use tokio::{net::TcpListener, select};
use tracing::{error, info, warn};

use crate::{
    host::{HostShared, OAuth2Manager, OAuth2State, PendingOAuth2Store, WebHost},
    routes::{RouteOptions, mk_routes},
};

pub struct Listeners {
    listeners: HashMap<SocketAddr, Listener>,
    kill_switch: Arc<AtomicBool>,
    shared: HostShared,
    oauth2_manager: Option<Arc<OAuth2Manager>>,
    route_options: RouteOptions,
}

impl Listeners {
    pub fn new(
        kill_switch: Arc<AtomicBool>,
        shared: HostShared,
        oauth2_manager: Option<Arc<OAuth2Manager>>,
        route_options: RouteOptions,
    ) -> (
        Self,
        tokio::sync::mpsc::Receiver<ListenersMessage>,
        ListenersClient,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let listeners = Self {
            listeners: HashMap::new(),
            kill_switch,
            shared,
            oauth2_manager,
            route_options,
        };
        let listeners_client = ListenersClient::new(tx);
        (listeners, rx, listeners_client)
    }

    pub async fn run(
        &mut self,
        mut listeners_channel: tokio::sync::mpsc::Receiver<ListenersMessage>,
    ) {
        loop {
            let message = select! {
                _ = wait_for_kill_switch(self.kill_switch.clone()) => {
                    info!("Host kill switch activated, stopping...");
                    return;
                }
                message = listeners_channel.recv() => message,
            };

            match message {
                Some(ListenersMessage::AddTlsListener(_, handler, addr, reply)) => {
                    error!(?addr, "TLS listeners not supported by web-host");
                    let _ = reply.send(Err(ListenersError::AddListenerFailed(handler, addr)));
                }
                Some(ListenersMessage::AddListener(kind, handler, addr, reply))
                    if kind != ListenerKind::HTTP =>
                {
                    error!(?addr, %kind, "The web host only opens http listeners");
                    let _ = reply.send(Err(ListenersError::AddListenerFailed(handler, addr)));
                }
                Some(ListenersMessage::AddListener(_, handler, addr, reply)) => {
                    self.add_listener(handler, addr, reply).await;
                }
                Some(ListenersMessage::RemoveListener(kind, addr, reply)) => {
                    if kind == ListenerKind::HTTP {
                        self.remove_listener(addr, reply);
                    } else {
                        let _ = reply.send(Err(ListenersError::RemoveListenerFailed(addr)));
                    }
                }
                Some(ListenersMessage::GetListeners(tx)) => {
                    self.send_listeners(tx);
                }
                None => {
                    if self.kill_switch.load(Ordering::Relaxed) {
                        info!("Listeners channel closed during shutdown, stopping...");
                    } else {
                        warn!("Listeners channel closed, stopping...");
                    }
                    return;
                }
            }
        }
    }

    async fn add_listener(
        &mut self,
        handler: Obj,
        addr: SocketAddr,
        reply: tokio::sync::oneshot::Sender<Result<(), ListenersError>>,
    ) {
        let listener = match TcpListener::bind(addr).await {
            Ok(listener) => listener,
            Err(e) => {
                let _ = reply.send(Err(ListenersError::AddListenerFailed(handler, addr)));
                error!(?addr, "Unable to bind listener: {}", e);
                return;
            }
        };

        let local_addr = match listener.local_addr() {
            Ok(addr) => addr,
            Err(e) => {
                error!(?addr, "Unable to get local address: {}", e);
                return;
            }
        };

        let web_host = WebHost::new(handler, local_addr.port(), self.shared.clone());
        let oauth2_state = self.oauth2_state(&web_host);
        let main_router = match mk_routes(web_host, oauth2_state, &self.route_options) {
            Ok(router) => router,
            Err(e) => {
                warn!(?e, "Unable to create main router");
                return;
            }
        };

        let (terminate_send, terminate_receive) = tokio::sync::watch::channel(false);
        self.listeners
            .insert(addr, Listener::new(terminate_send, handler));
        let _ = reply.send(Ok(()));

        tokio::spawn(async move {
            let mut term_receive = terminate_receive.clone();
            select! {
                _ = term_receive.changed() => {
                    info!("Listener terminated, stopping...");
                }
                _ = Listener::serve(listener, main_router) => {
                    info!("Listener exited, restarting...");
                }
            }
        });
    }

    fn oauth2_state(&self, web_host: &WebHost) -> Option<OAuth2State> {
        self.oauth2_manager.as_ref().map(|manager| {
            let pending = Arc::new(PendingOAuth2Store::new());
            let reap_pending = Arc::clone(&pending);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    reap_pending.reap_expired();
                }
            });
            OAuth2State {
                manager: Arc::clone(manager),
                web_host: web_host.clone(),
                pending,
            }
        })
    }

    fn remove_listener(
        &mut self,
        addr: SocketAddr,
        reply: tokio::sync::oneshot::Sender<Result<(), ListenersError>>,
    ) {
        let listener = self.listeners.remove(&addr);
        info!(?addr, "Removing listener");

        let Some(listener) = listener else {
            let _ = reply.send(Err(ListenersError::RemoveListenerFailed(addr)));
            return;
        };

        if let Err(e) = listener.terminate.send(true) {
            error!("Unable to send terminate message: {}", e);
        }
        let _ = reply.send(Ok(()));
    }

    fn send_listeners(&self, tx: tokio::sync::oneshot::Sender<Vec<ListenerInfo>>) {
        let listeners = self
            .listeners
            .iter()
            .map(|(addr, listener)| ListenerInfo {
                kind: ListenerKind::HTTP,
                handler: listener.handler_object,
                addr: *addr,
                is_tls: false,
            })
            .collect();
        if let Err(e) = tx.send(listeners) {
            error!("Unable to send listeners list: {:?}", e);
        }
    }
}

async fn wait_for_kill_switch(kill_switch: Arc<AtomicBool>) {
    while !kill_switch.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

pub struct Listener {
    handler_object: Obj,
    terminate: tokio::sync::watch::Sender<bool>,
}

impl Listener {
    fn new(terminate: tokio::sync::watch::Sender<bool>, handler_object: Obj) -> Self {
        Self {
            handler_object,
            terminate,
        }
    }

    async fn serve(listener: TcpListener, main_router: Router) -> eyre::Result<()> {
        let addr = listener.local_addr()?;
        info!("Listening on {:?}", addr);
        axum::serve(
            listener,
            main_router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        info!("Done listening on {:?}", addr);
        Ok(())
    }
}

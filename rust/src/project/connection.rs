use std::{fmt::Display, sync::Arc};

use futures::{Stream, StreamExt as _};
use samod::{ConnDirection, ConnFinishedReason, Repo};
use tokio::{
    net::TcpStream, select, sync::{broadcast, watch}
};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;

use crate::helpers::utils::spawn_named;

#[derive(Debug, Clone)]
enum ConnectionStoppedReason {
    TcpConnectionError(String),
    WebSocketsConnectionError(String),
    ConnectionCompleted(ConnFinishedReason),
    ConnectionCancelled()
}

impl Display for ConnectionStoppedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TcpConnectionError(e) => write!(f, "TCP connection error: {}", e),
            Self::WebSocketsConnectionError(e) => write!(f, "WebSocket connection error: {}", e),
            Self::ConnectionCompleted(reason) => write!(f, "Connection completed: {:?}", reason),
            Self::ConnectionCancelled() => write!(f, "Connection cancelled"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum RemoteConnectionEvent {
    ConnectionFailed,
    Connected,
    ConnectionCompleted,
}

#[derive(Debug, Clone)]
pub enum RemoteConnectionStatus {
    Connected,
    Disconnected,
}

/// Connects a repo to the remote server. Shuts down when dropped.
#[derive(Debug)]
pub struct RemoteConnection {
    status_rx: watch::Receiver<RemoteConnectionStatus>,
    events_tx: broadcast::Sender<RemoteConnectionEvent>,
    inner: Arc<RemoteConnectionInner>
}

#[derive(Clone, Debug)]
struct RemoteConnectionInner {
    repo: Repo,
    server_url: String,
    events_tx: broadcast::Sender<RemoteConnectionEvent>,
    status_tx: watch::Sender<RemoteConnectionStatus>,
    token: CancellationToken
}

impl Drop for RemoteConnection {
    // Stop the connection on drop
    fn drop(&mut self) {
        self.inner.token.cancel()
    }
}

impl RemoteConnection {
    /// Starts a connection to the server. A background task will be dispatched that
    /// reattempts connection indefinitely until the handle is dropped.
    pub fn new(repo: Repo, server_url: String) -> Self {
        let (events_tx, _) = broadcast::channel(32);
        let (status_tx, status_rx) = watch::channel(RemoteConnectionStatus::Disconnected);
        let inner = Arc::new(RemoteConnectionInner {
            repo,
            server_url,
            events_tx: events_tx.clone(),
            status_tx,
            token: CancellationToken::new()
        });

        let inner_clone = inner.clone();
        spawn_named("Remote connection", async move {
            inner_clone.retry_connection().await;
        });

        Self {
            inner,
            status_rx,
            events_tx
        }
    }

    /// Subscribe to future events.
    pub fn events(&self) -> impl Stream<Item = RemoteConnectionEvent> {
        let rx = self.events_tx.subscribe();
        BroadcastStream::new(rx).filter_map(|result| async move {
            match result {
                Ok(event) => Some(event),
                // Happens when the stream lags
                Err(err) => {
                    tracing::warn!("Dropped remote connection events: {:?}", err);
                    None
                }
            }
        })
    }

    /// Get the current status of the remote connection.
    pub fn status(&self) -> RemoteConnectionStatus {
        self.status_rx.borrow().clone()
    }
}

impl RemoteConnectionInner {
    async fn retry_connection(&self) {
        // The old code responded to different connection stop reason to change the backoff behavior.
        // If we want to keep doing that, add intelligent backoff here.
        // For now, this just tries once every second forever and ever.
        let backoff = 1000;
        loop {
            let termination_reason = self.try_connection().await;
            match termination_reason {
                ConnectionStoppedReason::ConnectionCancelled() => break,
                _ => (),
            }
            tracing::error!("Connection failure: {:?}", termination_reason);
            tracing::error!("Retrying in {}ms...", backoff);

            // Look for the cancelation token here as well, so we cancel from the backoff
            select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(backoff as u64)) => {}
                _ = self.token.cancelled() => {
                    break;
                }
            }
        }
    }

    async fn try_connection(&self) -> ConnectionStoppedReason {
        let repo_handle = self.repo.clone();
        let server_url = self.server_url.clone();

        tracing::info!("Attempting to connect to server at {server_url}...");

        // Connect via websockets
        let connection = if server_url.starts_with("ws://") {
            let res = tokio_tungstenite::connect_async(server_url.clone()).await;
            match res {
                Err(e) => {
                    _ = self.events_tx.send(RemoteConnectionEvent::ConnectionFailed);
                    return ConnectionStoppedReason::WebSocketsConnectionError(e.to_string());
                }
                Ok((res, _)) => repo_handle
                    .connect_tungstenite(res, ConnDirection::Outgoing)
                    .unwrap(),
            }
        }
        // Connect via TCP
        else {
            let res = TcpStream::connect(server_url.clone()).await;
            match res {
                Err(e) => {
                    _ = self.events_tx.send(RemoteConnectionEvent::ConnectionFailed);
                    return ConnectionStoppedReason::TcpConnectionError(e.to_string());
                }
                Ok(res) => repo_handle
                    .connect_tokio_io(res, ConnDirection::Outgoing)
                    .unwrap(),
            }
        };

        tracing::info!("Connected successfully!");

        if let Err(e) = connection.handshake_complete().await {
            _ = self.status_tx.send(RemoteConnectionStatus::Disconnected);
            _ = self.events_tx.send(RemoteConnectionEvent::ConnectionFailed);
            return ConnectionStoppedReason::ConnectionCompleted(e);
        }

        tracing::info!("Handshake completed!");
        _ = self.status_tx.send(RemoteConnectionStatus::Connected);
        _ = self.events_tx.send(RemoteConnectionEvent::Connected);
        
        select! {
            completed = connection.finished() => {
                _ = self.status_tx.send(RemoteConnectionStatus::Disconnected);
                _ = self.events_tx.send(RemoteConnectionEvent::ConnectionCompleted);
                return ConnectionStoppedReason::ConnectionCompleted(completed);
            },
            _ = self.token.cancelled() => {
                return ConnectionStoppedReason::ConnectionCancelled();
            }
        }
    }
}

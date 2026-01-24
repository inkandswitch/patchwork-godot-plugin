use futures::{Stream, StreamExt as _};
use samod::{ConnDirection, ConnFinishedReason, Repo};
use tokio::{
    net::TcpStream,
    sync::{broadcast, watch},
    task::JoinHandle,
};
use tokio_stream::wrappers::BroadcastStream;

#[derive(Debug, Clone)]
enum ConnectionStoppedReason {
    TcpConnectionError(String),
    WebSocketsConnectionError(String),
    ConnectionCompleted(ConnFinishedReason),
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

/// Connects a repo to the remote server.
#[derive(Debug)]
pub struct RemoteConnection {
    connection_handle: JoinHandle<()>,
    status_rx: watch::Receiver<RemoteConnectionStatus>,
    events_tx: broadcast::Sender<RemoteConnectionEvent>,
}

#[derive(Clone, Debug)]
struct RemoteConnectionInner {
    repo: Repo,
    server_url: String,
    events_tx: broadcast::Sender<RemoteConnectionEvent>,
    status_tx: watch::Sender<RemoteConnectionStatus>,
}

impl Drop for RemoteConnection {
    // Stop the connection on drop
    fn drop(&mut self) {
        &self.connection_handle.abort();
    }
}

impl RemoteConnection {
    /// Starts a connection to the server. A background task will be dispatched that
    /// reattempts connection indefinitely until the handle is dropped.
    pub fn new(repo: Repo, server_url: String) -> Self {
        let (events_tx, _) = broadcast::channel(32);
        let (status_tx, status_rx) = watch::channel(RemoteConnectionStatus::Disconnected);
        let inner = RemoteConnectionInner {
            repo,
            server_url,
            events_tx: events_tx.clone(),
            status_tx,
        };

        let connection_handle: JoinHandle<()> = tokio::spawn(async move {
            inner.retry_connection().await;
        });

        Self {
            connection_handle,
            status_rx,
            events_tx,
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
            tracing::error!("Connection failure: {:?}", termination_reason);
            tracing::error!("Retrying in {}ms...", backoff);
            tokio::time::sleep(std::time::Duration::from_millis(backoff as u64)).await;
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
                    self.events_tx.send(RemoteConnectionEvent::ConnectionFailed);
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
                    self.events_tx.send(RemoteConnectionEvent::ConnectionFailed);
                    return ConnectionStoppedReason::TcpConnectionError(e.to_string());
                }
                Ok(res) => repo_handle
                    .connect_tokio_io(res, ConnDirection::Outgoing)
                    .unwrap(),
            }
        };

        tracing::info!("Connected successfully!");

        if let Err(e) = connection.handshake_complete().await {
            self.status_tx.send(RemoteConnectionStatus::Disconnected);
            self.events_tx.send(RemoteConnectionEvent::ConnectionFailed);
            return ConnectionStoppedReason::ConnectionCompleted(e);
        }

        tracing::info!("Handshake completed!");
        self.status_tx.send(RemoteConnectionStatus::Connected);
        self.events_tx.send(RemoteConnectionEvent::Connected);
        
        let completed = connection.finished().await;
        self.status_tx.send(RemoteConnectionStatus::Disconnected);
        self.events_tx.send(RemoteConnectionEvent::ConnectionCompleted);
        return ConnectionStoppedReason::ConnectionCompleted(completed);
    }
}

use std::sync::Arc;

use crate::{project::branch_db::BranchDb};
use futures::StreamExt;
use samod::{Connection, ConnectionInfo, Repo};
use tokio::{sync::Mutex, task::JoinHandle};

#[derive(Debug)]
pub struct PeerWatcher {
    server_info: Arc<Mutex<Option<ConnectionInfo>>>,
    handle: JoinHandle<()>,
}

impl Drop for PeerWatcher {
    fn drop(&mut self) {
        // Is this safe? Alternatively we could use a cancellation token
        // I think it's safe though
        self.handle.abort();
    }
}

impl PeerWatcher {
    pub fn new(repo_handle: Repo, branch_db: BranchDb) -> Self {
        let server_info = Arc::new(Mutex::new(None));
        let server_info_clone = server_info.clone();
        let repo_handle_clone = repo_handle.clone();
        let handle = tokio::spawn(async move {
            let (_, stream) = repo_handle_clone.connected_peers();
            tokio::pin!(stream);
            while let Some(peers) = stream.next().await {
                // Currently, we only ever have 1 peer: the server.
                // Therefore, this code expects that the server is the first and only peer, if it's connected.
                // When we move to more peers, we'll need to figure out a way to identify the server here.
                if let Some(info) = peers.first() {
                    Self::update_server_info(server_info_clone.clone(), info.clone()).await;
                }
            }
        });

        Self {
            server_info,
            handle,
        }
    }

    pub async fn get_server_info(&self) -> Option<ConnectionInfo> {
        return self.server_info.lock().await.clone();
    }

    async fn update_server_info(
        old_info: Arc<Mutex<Option<ConnectionInfo>>>,
        new_info: ConnectionInfo,
    ) {
        let mut server_info = old_info.lock().await;
        if server_info.is_none() {
            *server_info = Some(new_info);
            return;
        }
        let mut info = server_info.clone().unwrap();
        info.last_received = new_info.last_received;
        info.last_sent = new_info.last_sent;

        for (doc_id, new_doc_state) in &new_info.docs {
            if let Some(old_doc_state) = info.docs.get(doc_id) {
                // If we got beheaded, skip this doc.
                if new_doc_state
                    .last_acked_heads
                    .as_ref()
                    .is_some_and(|h| h.len() == 0)
                    && old_doc_state
                        .last_acked_heads
                        .as_ref()
                        .is_some_and(|h| h.len() > 0)
                {
                    continue;
                }
            }
            info.docs.insert(doc_id.clone(), new_doc_state.clone());
        }

        *server_info = Some(info);
    }
}

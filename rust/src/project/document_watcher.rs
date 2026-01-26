use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::Arc,
};

use crate::{
    helpers::{
        branch::{
            BranchState, BranchStateForkInfo, BranchStateMergeInfo, BranchStateRevertInfo,
            BranchesMetadataDoc,
        },
        doc_utils::SimpleDocReader,
        utils::parse_automerge_url,
    },
    project::branch_db::BranchDb,
};
use automerge::{ChangeHash, ROOT, ReadDoc};
use autosurgeon::hydrate;
use futures::{FutureExt, StreamExt};
use samod::{DocHandle, DocumentId, Repo};
use tokio::select;
use tokio_util::sync::CancellationToken;

/// Tracks branch and metadata documents from an Automerge repo, updating BranchDB when the state changes.
#[derive(Debug)]
pub struct DocumentWatcher {
    inner: Arc<DocumentWatcherInner>,
}

#[derive(Debug, Clone)]
struct DocumentWatcherInner {
    repo: Repo,
    branch_db: BranchDb,
    token: CancellationToken,
}

impl Drop for DocumentWatcher {
    fn drop(&mut self) {
        self.inner.token.cancel();
    }
}

impl DocumentWatcher {
    /// Spawns the [DocumentWatcher], creating parallel tasks for the metadata document tracking and subsequent tasks for any child documents.
    pub async fn new(repo: Repo, branch_db: BranchDb, metadata_handle: DocHandle) -> Self {
        let inner = Arc::new(DocumentWatcherInner {
            branch_db,
            repo,
            token: CancellationToken::new(),
        });

        let inner_clone = inner.clone();

        // do the initial ingest
        inner_clone
            .ingest_metadata_document(metadata_handle.clone())
            .await;

        // track changes for future ingests
        tokio::spawn(async move {
            inner_clone.track_metadata_document(metadata_handle).await;
        });

        return Self { inner };
    }
}

impl DocumentWatcherInner {
    // The branch documents are a document for each branch, containing all the serialized data for all scenes and text files.
    async fn track_branch_document(&self, handle: DocHandle) {
        let mut stream = handle.changes();
        loop {
            select! {
                _ = stream.next() => {
                    // collapse the rest of the stream, in case multiple futures are ready
                    while let Some(_) = stream.next().now_or_never().flatten() {}
                    self.ingest_branch_document(handle.clone()).await;
                },
                _ = self.token.cancelled() => {
                    break;
                }
            }
        }
    }

    // The metadata document is the root document containing IDs of all branch docs.
    async fn track_metadata_document(&self, handle: DocHandle) {
        let mut stream = handle.changes();
        loop {
            select! {
                _ = stream.next() => {
                    // collapse the rest of the stream, in case multiple futures are ready
                    while let Some(_) = stream.next().now_or_never().flatten() {}
                    self.ingest_metadata_document(handle.clone()).await;
                },
                _ = self.token.cancelled() => {
                    break;
                }
            }
        }
    }

    #[tracing::instrument(skip_all)]
    async fn ingest_branch_document(&self, handle: DocHandle) {
        let (_, meta) = self.branch_db.get_metadata_state().await.expect(
            "Somehow, we haven't loaded a metadata doc, but we're ingesting a branch document?!?!",
        );

        let branch = meta
            .branches
            .get(&handle.document_id().to_string())
            .unwrap();

        // Create a default branch state, but only if we don't have an existing branch state.
        let h = handle.clone();
        self.branch_db
            .insert_branch_state_if_not_exists(handle.document_id().clone(), move || BranchState {
                name: branch.name.clone(),
                doc_handle: h.clone(),
                linked_doc_ids: HashSet::new(),
                synced_heads: Vec::new(),
                fork_info: match &branch.fork_info {
                    Some(fork_info) => Some(BranchStateForkInfo {
                        forked_from: DocumentId::from_str(&fork_info.forked_from).unwrap(),
                        forked_at: fork_info
                            .forked_at
                            .iter()
                            .map(|h| ChangeHash::from_str(h).unwrap())
                            .collect(),
                    }),
                    None => None,
                },
                merge_info: match &branch.merge_info {
                    Some(merge_info) => Some(BranchStateMergeInfo {
                        merge_into: DocumentId::from_str(&merge_info.merge_into).unwrap(),
                        merge_at: merge_info
                            .merge_at
                            .iter()
                            .map(|h| ChangeHash::from_str(h).unwrap())
                            .collect(),
                    }),
                    None => None,
                },
                is_main: h.document_id().to_string() == meta.main_doc_id,
                created_by: branch.created_by.clone(),
                merged_into: match &branch.merged_into {
                    Some(merged_into) => match DocumentId::from_str(&merged_into) {
                        Ok(merged_into) => Some(merged_into),
                        Err(_) => None,
                    },
                    None => None,
                },
                revert_info: match &branch.reverted_to {
                    Some(reverted_to) => Some(BranchStateRevertInfo {
                        reverted_to: reverted_to
                            .iter()
                            .map(|h| ChangeHash::from_str(h).unwrap())
                            .collect(),
                    }),
                    None => None,
                },
            })
            .await;

        let h = handle.clone();
        let linked_docs = tokio::task::spawn_blocking(move || {
            // Collect all linked doc IDs from this branch
            h.with_document(|d| {
                let files = match d.get_obj_id(ROOT, "files") {
                    Some(files) => files,
                    None => {
                        tracing::warn!("Failed to load files for branch doc {:?}", h.document_id());
                        return HashMap::new();
                    }
                };

                d.keys(&files)
                    .filter_map(|path| {
                        let file = match d.get_obj_id(&files, &path) {
                            Some(file) => file,
                            None => {
                                tracing::error!("Failed to load linked doc {:?}", path);
                                return None;
                            }
                        };

                        let url = match d.get_string(&file, "url") {
                            Some(url) => url,
                            None => {
                                return None;
                            }
                        };

                        parse_automerge_url(&url).map(|id| (path.clone(), id))
                    })
                    .collect::<HashMap<String, DocumentId>>()
            })
        })
        .await
        .unwrap();

        let branch_state_mutex = self
            .branch_db
            .get_branch_state(&handle.document_id())
            .await
            .unwrap();
        let mut branch_state = branch_state_mutex.lock().await;
        branch_state.linked_doc_ids = linked_docs.values().cloned().collect();
    }

    #[tracing::instrument(skip_all)]
    async fn ingest_metadata_document(&self, handle: DocHandle) {
        // TODO: Stop tracking removed branches
        // Find added branches, and begin tracking them
        let h = handle.clone();
        let meta = tokio::task::spawn_blocking(move || {
            // TODO: correct error handling on hydration failure; currently panics!
            let branches_metadata: BranchesMetadataDoc = h.with_document(|d| hydrate(d).unwrap());
            branches_metadata
        })
        .await
        .unwrap();
        self.branch_db
            .set_metadata_state(handle.document_id().clone(), meta.clone())
            .await;
        // check if there are new branches that haven't loaded yet
        for (branch_id_str, _) in meta.branches.iter() {
            let branch_id = DocumentId::from_str(branch_id_str).unwrap();

            if !self.branch_db.has_branch(&branch_id).await {
                let Some(handle) = self.repo.find(branch_id.clone()).await.unwrap() else {
                    tracing::error!(
                        "Document {:?} exists in the branch metadata document, but not the repo! Skipping.",
                        branch_id
                    );
                    continue;
                };
                self.ingest_branch_document(handle.clone()).await;
                // Track the document
                let this = self.clone();
                tokio::spawn(async move { this.track_branch_document(handle).await });
            }
        }
    }
}

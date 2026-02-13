use std::collections::HashMap;

use automerge::Automerge;
use autosurgeon::{hydrate, reconcile};
use samod::{DocHandle, DocumentId};
use tracing::instrument;

use crate::{
    helpers::{
        branch::{Branch, BranchesMetadataDoc, ForkInfo, GodotProjectDoc},
        utils::{CommitMetadata, commit_with_metadata},
    },
    project::branch_db::{BranchDb, HistoryRef},
};

// Methods related to branch and document management on a [BranchDb].
impl BranchDb {
    /// Create a new metadata document, and a new main branch, and return the handle of the metadata document.
    /// Checks out the initial commit of the main branch automatically.
    pub async fn create_metadata_doc(&self) -> DocHandle {
        tracing::info!("Creating new metadata doc...");
        let username = self.username.lock().await.clone();

        // Because we always change the checked out ref after creating, we need to lock this in write mode.
        let r = self.get_checked_out_ref_mut();
        let mut checked_out_ref = r.write().await;

        // Create new main branch doc
        let main_handle = self.repo.create(Automerge::new()).await.unwrap();
        let main_handle_clone = main_handle.clone();
        let username_clone = username.clone();

        let new_heads = tokio::task::spawn_blocking(move || {
            main_handle_clone.with_document(|d| {
                let mut tx = d.transaction();
                let _ = reconcile(
                    &mut tx,
                    GodotProjectDoc {
                        files: HashMap::new(),
                        state: HashMap::new(),
                    },
                );
                commit_with_metadata(
                    tx,
                    &CommitMetadata {
                        username: username_clone,
                        branch_id: Some(main_handle_clone.document_id().clone()),
                        merge_metadata: None,
                        reverted_to: None,
                        changed_files: None,
                        is_setup: Some(true),
                    },
                );
                d.get_heads()
            })
        })
        .await
        .unwrap();

        *checked_out_ref = Some(HistoryRef {
            branch: main_handle.document_id().clone(),
            heads: new_heads,
        });

        let main_branch_doc_id = main_handle.document_id().to_string();
        let main_branch_doc_id_clone = main_branch_doc_id.clone();
        let branches = HashMap::from([(
            main_branch_doc_id,
            Branch {
                name: String::from("main"),
                id: main_handle.document_id().to_string(),
                fork_info: None,
                merge_info: None,
                created_by: username.clone(),
                reverted_to: None,
            },
        )]);
        let branches_clone = branches.clone();

        // create new branches metadata doc
        let metadata_handle = self.repo.create(Automerge::new()).await.unwrap();
        let metadata_handle_clone = metadata_handle.clone();
        tokio::task::spawn_blocking(move || {
            metadata_handle.with_document(|d| {
                let mut tx = d.transaction();
                let _ = reconcile(
                    &mut tx,
                    BranchesMetadataDoc {
                        main_doc_id: main_branch_doc_id_clone,
                        branches: branches_clone,
                    },
                );
                commit_with_metadata(
                    tx,
                    &CommitMetadata {
                        username: username,
                        branch_id: None,
                        merge_metadata: None,
                        reverted_to: None,
                        changed_files: None,
                        is_setup: Some(true),
                    },
                );
            });
        })
        .await
        .unwrap();
        metadata_handle_clone
    }

    #[instrument(skip_all)]
    pub(super) async fn add_branch_to_meta(&self, branch: Branch) {
        let meta_handle = {
            let meta = self.metadata_state.lock().await;
            if meta.is_none() {
                tracing::error!("Could not find metadata document!");
                return;
            }
            meta.as_ref().unwrap().0.clone()
        };

        let username = self.username.lock().await.clone();
        tokio::task::spawn_blocking(move || {
            meta_handle.with_document(|d| {
                let mut branches_metadata: BranchesMetadataDoc = hydrate(d).unwrap();
                let mut tx = d.transaction();
                branches_metadata.branches.insert(branch.id.clone(), branch);
                let _ = reconcile(&mut tx, branches_metadata);
                commit_with_metadata(
                    tx,
                    &CommitMetadata {
                        username: username,
                        branch_id: None,
                        merge_metadata: None,
                        reverted_to: None,
                        changed_files: None,
                        is_setup: Some(true),
                    },
                );
            });
        });
    }

    async fn remove_branch_from_meta(&self, branch: DocumentId) {
        let meta_handle = {
            let meta = self.metadata_state.lock().await;
            if meta.is_none() {
                tracing::error!("Could not find metadata document!");
                return;
            }
            meta.as_ref().unwrap().0.clone()
        };
        let branch_clone = branch.clone();
        let username = self.username.lock().await.clone();
        tokio::task::spawn_blocking(move || {
            meta_handle.with_document(|d| {
                let mut tx = d.transaction();
                let mut branches_metadata: BranchesMetadataDoc = hydrate(&mut tx).unwrap();
                branches_metadata.branches.remove(&branch_clone.to_string());
                let _ = reconcile(&mut tx, branches_metadata);
                commit_with_metadata(
                    tx,
                    &CommitMetadata {
                        username: username,
                        branch_id: None,
                        merge_metadata: None,
                        reverted_to: None,
                        changed_files: None,
                        is_setup: Some(true),
                    },
                );
            });
        })
        .await
        .unwrap();
    }

    // delete branch isn't fully implemented right now deletes are not propagated to the frontend
    // right now this is just useful to clean up merge preview branches
    #[instrument(skip_all)]
    pub async fn delete_branch(&self, branch: &DocumentId) {
        self.remove_branch_from_meta(branch.clone()).await;
    }

    pub(super) async fn clone_doc(&self, handle: DocHandle) -> DocHandle {
        let new_handle = self.repo.create(Automerge::new()).await.unwrap();

        let new_handle_clone = new_handle.clone();
        tokio::task::spawn_blocking(move || {
            handle.with_document(|mut main_d| {
                new_handle_clone
                    .with_document(|d| d.merge(&mut main_d))
                    .unwrap();
            });
        })
        .await
        .unwrap();

        return new_handle;
    }

    // TODO: This would be more versatile if we gave a HistoryRef instead of a branch.
    // That way it might work for reverts too?
    pub async fn fork_branch(&self, name: String, source: &DocumentId) -> Option<DocumentId> {
        tracing::info!("Forking new branch {:?} from source {:?}", name, source);
        let Some(source_handle) = self.get_branch_handle(source).await else {
            tracing::error!("Couldn't fork branch; existing source branch doesn't exist!");
            return None;
        };

        let Some(latest_ref) = self.get_latest_ref_on_branch(source).await else {
            tracing::error!("Couldn't get latest ref on source branch!");
            return None;
        };
        let new_handle = self.clone_doc(source_handle).await;
        let username = self.username.lock().await.clone();
        let id = new_handle.document_id();

        self.add_branch_to_meta(Branch {
            name: name.clone(),
            id: id.to_string(),
            fork_info: Some(ForkInfo {
                forked_from: source.to_string(),
                forked_at: latest_ref
                    .heads
                    .into_iter()
                    .map(|h| h.to_string())
                    .collect(),
            }),
            merge_info: None,
            created_by: username,
            reverted_to: None,
        })
        .await;
        Some(id.clone())
    }
}

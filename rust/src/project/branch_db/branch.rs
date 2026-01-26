use std::collections::HashMap;

use automerge::Automerge;
use autosurgeon::reconcile;
use samod::DocHandle;

use crate::{
    helpers::{
        branch::{Branch, BranchesMetadataDoc, GodotProjectDoc},
        utils::{CommitMetadata, commit_with_attribution_and_timestamp},
    },
    project::branch_db::{BranchDb, HistoryRef},
};

// Methods related to branch and document management on a [BranchDb].
impl BranchDb {
    /// Create a new metadata document, and a new main branch, and return the handle of the metadata document.
    /// Checks out the initial commit of the main branch automatically.
    pub async fn create_metadata_doc(&self) -> DocHandle {
        tracing::info!("Creating new metadata doc...");
        let repo = self.inner.lock().await.repo.clone();
        let username = self.inner.lock().await.username.clone();
        
        // Because we always change the checked out ref after creating, we need to lock this in write mode.
        let r = self.get_checked_out_ref_mut().await;
        let mut checked_out_ref = r.write().await;

        // Create new main branch doc
        let main_handle = repo.create(Automerge::new()).await.unwrap();
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
                commit_with_attribution_and_timestamp(
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
            heads: new_heads
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
                merged_into: None,
                reverted_to: None,
            },
        )]);
        let branches_clone = branches.clone();

        // create new branches metadata doc
        let metadata_handle = repo.create(Automerge::new()).await.unwrap();
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
                commit_with_attribution_and_timestamp(
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
}

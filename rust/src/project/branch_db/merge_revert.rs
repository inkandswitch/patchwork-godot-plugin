use automerge::{Automerge, ROOT, transaction::Transactable};
use autosurgeon::{hydrate, reconcile};
use samod::DocumentId;
use tracing::instrument;

use crate::{
    helpers::{
        branch::{Branch, BranchesMetadataDoc, ForkInfo, MergeInfo},
        doc_utils::SimpleDocReader,
        utils::{CommitMetadata, MergeMetadata, commit_with_metadata},
    },
    project::branch_db::{BranchDb, branch},
};

impl BranchDb {
    #[instrument(skip_all)]
    pub async fn create_merge_preview_branch(
        &self,
        source: &DocumentId,
        target: &DocumentId,
    ) -> Option<DocumentId> {
        // Not getting the branch state so we don't gotta clone, honestly that was probably simpler though
        let source_name = self.get_branch_name(source).await?;
        let target_name = self.get_branch_name(target).await?;
        let source_handle = self.get_branch_handle(source).await?;
        let target_handle = self.get_branch_handle(target).await?;
        
        let source_ref = self.get_latest_ref_on_branch(source).await?;
        let target_ref = self.get_latest_ref_on_branch(target).await?;

        let handle = self.repo.create(Automerge::new()).await.unwrap();
        let handle_clone = handle.clone();

        tokio::task::spawn_blocking(move || {
            source_handle.with_document(|d| {
                handle_clone.with_document(|preview_doc| {
                    let _ = preview_doc.merge(d);
                });
            });

            target_handle.with_document(|d| {
                handle_clone.with_document(|preview_doc| {
                    let _ = preview_doc.merge(d);
                });
            });
        })
        .await
        .unwrap();

        let username = self.username.lock().await.clone();
        self.add_branch_to_meta(Branch {
            name: format!("{} <- {}", target_name, source_name),
            id: handle.document_id().to_string(),
            fork_info: Some(ForkInfo {
                forked_from: source.to_string(),
                forked_at: source_ref
                    .heads
                    .iter()
                    .map(|h| h.to_string())
                    .collect(),
            }),
            merge_info: Some(MergeInfo {
                merge_into: target.to_string(),
                merge_at: target_ref
                    .heads
                    .iter()
                    .map(|h| h.to_string())
                    .collect(),
            }),
            created_by: username.clone(),
            reverted_to: None,
        })
        .await;
        Some(handle.document_id().clone())
    }

    pub async fn merge_branch(&self, source: &DocumentId, target: &DocumentId) {
        let Some(source_state) = self.get_branch_state(source).await else {
            return;
        };
        let Some(target_state) = self.get_branch_state(target).await else {
            return;
        };

        let source_handle = source_state.doc_handle.clone();
        let target_handle = target_state.doc_handle.clone();
        tokio::task::spawn_blocking(move || {
            source_handle.with_document(|d| {
                target_handle.with_document(|target| {
                    let _ = target.merge(d);
                });
            });
        });

        // if the branch has some merge_info we know that it's a merge preview branch
        let merge_metadata = if source_state.merge_info.is_some() {
            match self
                .get_branch_state(&source_state.fork_info.as_ref().unwrap().forked_from)
                .await
            {
                Some(original_state) => {
                    Some(MergeMetadata {
                        merged_branch_id: original_state.doc_handle.document_id().clone(),
                        merged_at_heads: original_state.synced_heads.clone(),
                        forked_at_heads: original_state
                            .fork_info
                            .as_ref()
                            .unwrap()
                            .forked_at
                            .clone(),
                    })
                }
                _ => None,
            }
        } else {
            // todo: implement this case
            None
        };

        let username = self.username.lock().await.clone();
        if let Some(merge_metadata) = merge_metadata {
            let target = target.clone();
            let target_handle = target_state.doc_handle.clone();
            tokio::task::spawn_blocking(move || {
                target_handle.with_document(|d| {
                    let mut tx = d.transaction();

                    // do a dummy change that we can attach some metadata to
                    let changed = tx.get_int(&ROOT, "_changed").unwrap_or(0);
                    let _ = tx.put(ROOT, "_changed", changed + 1);

                    commit_with_metadata(
                        tx,
                        &CommitMetadata {
                            username: username.clone(),
                            branch_id: Some(target.clone()),
                            merge_metadata: Some(merge_metadata),
                            reverted_to: None,
                            changed_files: None,
                            is_setup: Some(false),
                        },
                    );
                });
            })
            .await
            .unwrap();
        }
    }
}

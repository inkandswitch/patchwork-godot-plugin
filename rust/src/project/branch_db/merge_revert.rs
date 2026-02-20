use automerge::{Automerge, ROOT, transaction::Transactable};
use samod::DocumentId;
use tracing::instrument;

use crate::{
    helpers::{
        branch::Branch,
        doc_utils::SimpleDocReader,
        utils::{CommitMetadata, MergeMetadata, commit_with_metadata},
    },
    project::branch_db::BranchDb,
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

        let source_ref = self.get_latest_ref_on_branch(source).await?;
        let target_ref = self.get_latest_ref_on_branch(target).await?;

        let handle = self.repo.create(Automerge::new()).await.unwrap();
        let handle_clone = handle.clone();

        self.with_shadow_document(source, async |d| {
            handle_clone.with_document(|preview_doc| {
                let _ = preview_doc.merge(d);
            });
        })
        .await
        .ok()?;

        self.with_shadow_document(target, async |d| {
            handle_clone.with_document(|preview_doc| {
                let _ = preview_doc.merge(d);
            });
        })
        .await
        .ok()?;

        let username = self.username.lock().await.clone();
        self.add_branch_to_meta(Branch {
            name: format!("{} <- {}", target_name, source_name),
            id: handle.document_id().clone(),
            forked_from: Some(source_ref),
            merge_into: Some(target_ref),
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

        if source == target {
            tracing::error!("cannot merge branch into itself!");
            return;
        }

        self.with_shadow_document(source, async |source_doc| {
            self.with_shadow_document(target, async |target_doc| {
                let _ = target_doc.merge(source_doc);
            })
            .await.unwrap();
        })
        .await.unwrap();

        // if the branch has some merge_into we know that it's a merge preview branch
        // forked_from is the original branch of the preview branch
        let forked_from = source_state.forked_from.unwrap().branch().clone();
        let merge_metadata = if source_state.merge_into.is_some() {
            match self.get_branch_state(&forked_from).await {
                Some(original_state) => Some(MergeMetadata {
                    merged_branch_id: forked_from,
                    forked_at_heads: original_state.forked_from.unwrap().heads().clone(),
                }),
                _ => None,
            }
        } else {
            // todo: implement this case
            None
        };

        let username = self.username.lock().await.clone();
        if let Some(merge_metadata) = merge_metadata {
            let target = target.clone();
            self.with_shadow_document(&target, async |d| {
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
            })
            .await.unwrap();
        }
    }
}

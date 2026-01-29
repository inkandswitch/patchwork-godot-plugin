use std::{path::PathBuf, str::FromStr};

use samod::{DocHandle, DocumentId};
use tracing::instrument;

use crate::{helpers::branch::BranchState, project::branch_db::{BranchDb, HistoryRef}};

// Utility methods for working with [BranchDb].
impl BranchDb {
    /// Turns a filesytem path into a project-local res:// path.
    /// Local paths are represented with a [String], while global paths are represented with a [PathBuf].
    /// This is because local paths are a URL, not a filesystem path.
    pub fn localize_path(&self, path: &PathBuf) -> String {
        let path = path.to_string_lossy().replace("\\", "/");
        let project_dir = self.project_dir.to_string_lossy().replace("\\", "/");
        if path.starts_with(&project_dir) {
            // TODO: this isn't teeechnically a Path, it's a URL... PathBuf is probably the wrong choice.
            // That's why we turn it into a string when we export!
            let thing = PathBuf::from("res://".to_string())
                .join(PathBuf::from(&path[project_dir.len()..].to_string()));
            thing.to_string_lossy().to_string()
        } else {
            path.to_string()
        }
    }

    /// Convert a project URL like res:// into a local filesystem path.
    /// Local paths are represented with a [String], while global paths are represented with a [PathBuf].
    /// This is because local paths are a URL, not a filesystem path.
    pub fn globalize_path(&self, path: &String) -> PathBuf {
        // trim the project_dir from the front of the path
        if path.starts_with("res://") {
            self.project_dir.clone().join(&path["res://".len()..])
        } else {
            PathBuf::from(path)
        }
    }

    /// Get the most recent ref on a given branch.
    // TODO (Lilith): This replaces branch_state.synced_heads. Either remove synced_heads,
    // or figure out a way to reliably update it when the heads actually change.
    // In the old system, synced heads was just force-updated every branch update.
    // Maybe that's enough? Get DocumentWatcher to do it? Then we remove the with_doc call here. 
    #[instrument(skip_all)]
    pub async fn get_latest_ref_on_branch(&self, branch: &DocumentId) -> Option<HistoryRef> {
        let handle = self.get_branch_handle(branch).await?;
        let heads = tokio::task::spawn_blocking(move || handle.with_document(|d| d.get_heads())).await.unwrap();
        
        Some(HistoryRef {
            heads,
            branch: branch.clone()
        })
    }

    pub async fn get_main_branch(&self) -> Option<DocumentId> {
        let Some((_, metadata)) = self.get_metadata_state().await else {
            tracing::error!("Couldn't get main branch; no metadata doc.");
            return None;
        };
        // TODO (Lilith): Figure out a way to hydrate/reconcile DocumentID so we don't have to do the string parse here.
        // Alternatively, don't store BranchesMetadataDoc, store some similar thing to BranchState
        return DocumentId::from_str(&metadata.main_doc_id).ok();
    }

    /// Check if a path should be ignored based on the provided glob patterns
    pub fn should_ignore(&self, path: &PathBuf) -> bool {
        // TODO: We should check if it's a symlink or not, but right now it's sufficient to just check if it's outside of the watch path
        // check if it's outside of the watch path
        if path.is_symlink() {
            return true;
        }
        if !path.starts_with(&self.project_dir) {
            return true;
        }
        self.ignore_globs
            .iter()
            .any(|pattern| pattern.matches(&path.to_string_lossy()))
    }
    
    pub async fn get_branch_name(&self, id: &DocumentId) -> Option<String> {
        let states = self.branch_states.lock().await;
        Some(states.get(id)?.lock().await.name.clone())
    }

    // This is not ideal -- I'd prefer not to clone unless necessary.
    // However, we NEVER want to expose our internal BranchState mutexes.
    // That could cause deadlocks if they acquired a branch state and later tried to call any branch info method on branch_db.
    // Callers should preferentially use other getter methods.
    pub async fn get_branch_state(&self, id: &DocumentId) -> Option<BranchState> {
        let states = self.branch_states.lock().await;
        Some(states.get(id)?.lock().await.clone())
    }

    pub async fn get_branch_handle(&self, id: &DocumentId) -> Option<DocHandle> {
        let states = self.branch_states.lock().await;
        Some(states.get(id)?.lock().await.doc_handle.clone())
    }

    pub async fn get_branch_children(&self, id: &DocumentId) -> Vec<DocumentId> {
        let states = self.branch_states.lock().await;
        let mut result = Vec::new();

        for (bid, state) in states.iter() {
            let state: tokio::sync::MutexGuard<'_, crate::helpers::branch::BranchState> = state.lock().await;
            if let Some(fork_info) = &state.fork_info {
                if &fork_info.forked_from == id {
                    result.push(bid.clone());
                }
            }
        }
        result
    }
}
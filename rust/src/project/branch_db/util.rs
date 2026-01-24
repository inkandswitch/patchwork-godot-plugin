use std::{path::PathBuf, str::FromStr};

use samod::DocumentId;

use crate::project::branch_db::{BranchDb, HistoryRef};

// Utility methods for working with [BranchDb].
impl BranchDb {
    /// Turns a filesytem path into a project-local res:// path.
    pub fn localize_path(&self, path: &PathBuf) -> String {
        let path = path.to_string_lossy().replace("\\", "/");
        let project_dir = self.project_dir.to_string_lossy().replace("\\", "/");
        if path.starts_with(&project_dir) {
            // TODO: this isn't teeechnically a Path, it's a URL... PathBuf is probably the wrong choice
            let thing = PathBuf::from("res://".to_string())
                .join(PathBuf::from(&path[project_dir.len()..].to_string()));
            thing.to_string_lossy().to_string()
        } else {
            path.to_string()
        }
    }

    /// Convert a project URL like res:// into a local filesystem path.
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
    pub async fn get_latest_ref_on_branch(&self, branch: &DocumentId) -> Option<HistoryRef> {
        let state = self.get_branch_state(branch).await;
        let Some(state) = state else {
            tracing::error!("Couldn't get latest ref on branch; branch state not loaded!");
            return None;
        };
        let state = state.lock().await;
        let handle = state.doc_handle.clone();
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
    
}
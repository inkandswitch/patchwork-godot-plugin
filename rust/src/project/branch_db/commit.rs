use std::collections::{HashMap, HashSet};

use automerge::{Automerge, ChangeHash, ObjId, ObjType, ROOT, ReadDoc};
use autosurgeon::Doc;
use samod::{DocHandle, DocumentId};

use crate::{
    fs::file_utils::FileContent,
    helpers::{
        doc_utils::SimpleDocReader,
        utils::{
            ChangeType, ChangedFile, CommitMetadata, commit_with_attribution_and_timestamp,
            heads_to_vec_string,
        },
    },
    parser::godot_parser::GodotScene,
    project::branch_db::{BranchDb, HistoryRef},
};

// Methods related to committing changes to a branch in [BranchDb].
impl BranchDb {
    /// Commit a list of files from the filesystem, while ensuring they've actually been changed before including them.
    pub async fn commit_fs_changes(
        &self,
        files: Vec<(String, FileContent)>,
        ref_: &HistoryRef,
        revert: Option<Vec<ChangeHash>>,
        is_checking_in: bool,
    ) -> Option<HistoryRef> {
        let Some(branch_handle) = self.get_branch_handle(&ref_.branch).await else {
            return None;
        };

        // Only commit files that have actually changed
        let files = self.filter_changed_files(ref_, files).await;
        let username = self.inner.lock().await.username.clone();

        let mut binary_entries: Vec<(String, DocHandle)> = Vec::new();
        let mut text_entries: Vec<(String, String)> = Vec::new();
        let mut scene_entries: Vec<(String, GodotScene)> = Vec::new();
        let mut deleted_entries: Vec<String> = Vec::new();

        for (path, content) in files {
            match content {
                FileContent::Binary(content) => {
                    let handle = self.create_new_binary_doc(content).await;
                    binary_entries.push((path, handle));
                }
                FileContent::String(content) => {
                    text_entries.push((path, content));
                }
                FileContent::Scene(godot_scene) => {
                    scene_entries.push((path, godot_scene));
                }
                FileContent::Deleted => {
                    deleted_entries.push(path);
                }
            }
        }

        let new_heads = tokio::task::spawn_blocking(move || {
            branch_handle.with_document(|d| {
                // We currently only ever save files to the current heads
                let mut tx = d.transaction();

                let mut changes: Vec<ChangedFile> = Vec::new();
                let files = tx.get_obj_id(ROOT, "files").unwrap();

                // write text entries to doc
                for (path, content) in text_entries {
                    // get existing file url or create new one
                    let (file_entry, change_type) = match tx.get(&files, &path) {
                        Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => {
                            (file_entry, ChangeType::Modified)
                        }
                        _ => (
                            tx.put_object(&files, &path, ObjType::Map).unwrap(),
                            ChangeType::Added,
                        ),
                    };

                    changes.push(ChangedFile { path, change_type });

                    // delete url in file entry if it previously had one
                    if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
                        let _ = tx.delete(&file_entry, "url");
                    }

                    // delete structured content in file entry if it previously had one
                    if let Ok(Some((_, _))) = tx.get(&file_entry, "structured_content") {
                        let _ = tx.delete(&file_entry, "structured_content");
                    }

                    // either get existing text or create new text
                    let content_key = match tx.get(&file_entry, "content") {
                        Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                        _ => tx
                            .put_object(&file_entry, "content", ObjType::Text)
                            .unwrap(),
                    };
                    let _ = tx.update_text(&content_key, &content);
                }

                // write scene entries to doc
                for (path, godot_scene) in scene_entries {
                    // get the change flag
                    let change_type = match tx.get(&files, &path) {
                        Ok(Some(_)) => ChangeType::Modified,
                        _ => ChangeType::Added,
                    };

                    let scene_file = tx
                        .get_obj_id(&files, &path)
                        .unwrap_or_else(|| tx.put_object(&files, &path, ObjType::Map).unwrap());
                    autosurgeon::reconcile_prop(
                        &mut tx,
                        &scene_file,
                        "structured_content",
                        godot_scene,
                    )
                    .unwrap_or_else(|e| {
                        tracing::error!("error reconciling scene: {}", e);
                        panic!("error reconciling scene: {}", e);
                    });
                    changes.push(ChangedFile { path, change_type });
                }

                // write binary entries to doc
                for (path, binary_doc_handle) in binary_entries {
                    // get the change flag
                    let change_type = match tx.get(&files, &path) {
                        Ok(Some(_)) => ChangeType::Modified,
                        _ => ChangeType::Added,
                    };

                    let file_entry = tx.put_object(&files, &path, ObjType::Map);
                    let _ = tx.put(
                        file_entry.unwrap(),
                        "url",
                        format!("automerge:{}", &binary_doc_handle.document_id()),
                    );

                    changes.push(ChangedFile { path, change_type });
                }

                for path in deleted_entries {
                    let _ = tx.delete(&files, &path);
                    changes.push(ChangedFile {
                        path,
                        change_type: ChangeType::Removed,
                    });
                }

                commit_with_attribution_and_timestamp(
                    tx,
                    &CommitMetadata {
                        username: username.clone(),
                        branch_id: Some(branch_handle.document_id().clone()),
                        merge_metadata: None,
                        reverted_to: match revert {
                            Some(revert) => Some(heads_to_vec_string(revert)),
                            None => None,
                        },
                        changed_files: Some(changes),
                        is_setup: Some(is_checking_in),
                    },
                );

                // TODO: I actually have no idea if this works -- do we need a new with_document
                // to check for new heads?
                d.get_heads()
            })
        })
        .await
        .unwrap();

        assert!(new_heads != ref_.heads);
        return Some(HistoryRef {
            heads: new_heads,
            branch: ref_.branch.clone(),
        });
    }

    // Filter a list of files to those changed compared to a given ref.
    async fn filter_changed_files(
        &self,
        ref_: &HistoryRef,
        files: Vec<(String, FileContent)>,
    ) -> Vec<(String, FileContent)> {
        // Only load files matching those we've provided
        let filter = files
            .iter()
            .map(|(path, _)| path.to_string())
            .collect::<HashSet<String>>();

        // Check our stored files
        let stored_files = self
            .get_files_at_ref(&ref_, &filter)
            .await
            .unwrap_or(HashMap::new());

        // Filter out files that haven't actually changed
        files
            .into_iter()
            .filter_map(|(path, content)| {
                let path = path.to_string();
                let stored_content = stored_files.get(&path);
                if let Some(stored_content) = stored_content {
                    if stored_content == &content {
                        return None;
                    }
                }
                Some((path, content))
            })
            .collect()
    }
    

    pub async fn create_new_binary_doc(&self, content: Vec<u8>) -> DocHandle {
        let handle = self
            .inner
            .lock()
            .await
            .repo
            .create(Automerge::new())
            .await
            .unwrap();

        let username = self.inner.lock().await.username.clone();

        // we're allowed to transact in the background: nobody needs this to exist yet.
        let h = handle.clone();
        tokio::task::spawn_blocking(move || {
            h.with_document(|d| {
                let mut tx = d.transaction();
                let _ = tx.put(ROOT, "content", content);
                commit_with_attribution_and_timestamp(
                    tx,
                    &CommitMetadata {
                        username: username,
                        branch_id: None,
                        merge_metadata: None,
                        reverted_to: None,
                        changed_files: None,
                        is_setup: Some(false),
                    },
                );
            });
        });

        // TODO: actually store the handle
        return handle;
    }
}

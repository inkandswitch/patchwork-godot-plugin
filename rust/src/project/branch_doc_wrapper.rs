use crate::helpers::branch::BranchState;
use crate::fs::file_utils::FileContent;
use crate::helpers::doc_utils::SimpleDocReader;
use crate::parser::godot_parser::GodotScene;
use crate::helpers::utils::{
    ChangeType, ChangedFile, CommitMetadata, commit_with_attribution_and_timestamp,
    get_default_patch_log, heads_to_vec_string, get_linked_docs_of_branch,
};
use automerge::{Automerge, ChangeHash, ObjId, ObjType, ReadDoc, ROOT, transaction::Transactable};
use samod::{DocHandle, DocumentId, Repo};
use autosurgeon::reconcile_prop;
use std::collections::{HashMap, HashSet};
use tracing::instrument;

#[derive(Debug, Clone)]
pub struct BranchDocWrapper {
    pub doc_handle: DocHandle,
    pub branch_state: BranchState,
}

impl BranchDocWrapper {
    pub fn new(doc_handle: DocHandle, branch_state: BranchState) -> Self {
        Self {
            doc_handle,
            branch_state,
        }
    }

    pub async fn save_files(
        &mut self,
        repo_handle: &Repo,
        file_entries: Vec<(String, FileContent)>,
        user_name: Option<String>,
        heads: Option<Vec<ChangeHash>>,
        new_project: bool,
        revert: Option<Vec<ChangeHash>>,
    ) -> Vec<(String, DocHandle)> {
        let mut binary_entries: Vec<(String, DocHandle)> = Vec::new();
        let mut text_entries: Vec<(String, &String)> = Vec::new();
        let mut scene_entries: Vec<(String, &GodotScene)> = Vec::new();
        let mut deleted_entries: Vec<String> = Vec::new();

        // Separate files by type
        for (path, content) in file_entries.iter() {
            match content {
                FileContent::Binary(content) => {
                    // Create a new binary document for this binary file
                    let binary_doc_handle = repo_handle.create(Automerge::new()).await.unwrap();
                    binary_doc_handle.with_document(|d| {
                        let mut tx = d.transaction();
                        let _ = tx.put(ROOT, "content", content.clone());
                        commit_with_attribution_and_timestamp(
                            tx,
                            &CommitMetadata {
                                username: user_name.clone(),
                                branch_id: None,
                                merge_metadata: None,
                                reverted_to: None,
                                changed_files: None,
                                is_setup: Some(new_project),
                            },
                        );
                    });

                    binary_entries.push((path.clone(), binary_doc_handle));
                }
                FileContent::String(content) => {
                    text_entries.push((path.clone(), content));
                }
                FileContent::Scene(godot_scene) => {
                    scene_entries.push((path.clone(), godot_scene));
                }
                FileContent::Deleted => {
                    deleted_entries.push(path.clone());
                }
            }
        }

        // Write all changes to the branch document
        self.doc_handle.with_document(|d| {
            let mut tx = match heads {
                Some(heads) => d.transaction_at(get_default_patch_log(), &heads),
                None => d.transaction(),
            };

            let mut changes: Vec<ChangedFile> = Vec::new();
            let files = tx.get_obj_id(ROOT, "files").unwrap();

            // Write text entries to doc
            for (path, content) in text_entries {
                // Get existing file entry or create new one
                let (file_entry, change_type) = match tx.get(&files, &path) {
                    Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => {
                        (file_entry, ChangeType::Modified)
                    }
                    _ => (
                        tx.put_object(&files, &path, ObjType::Map).unwrap(),
                        ChangeType::Added,
                    ),
                };

                changes.push(ChangedFile {
                    path: path.clone(),
                    change_type,
                });

                // Delete url in file entry if it previously had one
                if let Ok(Some((_, _))) = tx.get(&file_entry, "url") {
                    let _ = tx.delete(&file_entry, "url");
                }

                // Delete structured content in file entry if it previously had one
                if let Ok(Some((_, _))) = tx.get(&file_entry, "structured_content") {
                    let _ = tx.delete(&file_entry, "structured_content");
                }

                // Either get existing text or create new text
                let content_key = match tx.get(&file_entry, "content") {
                    Ok(Some((automerge::Value::Object(ObjType::Text), content))) => content,
                    _ => tx
                        .put_object(&file_entry, "content", ObjType::Text)
                        .unwrap(),
                };
                let _ = tx.update_text(&content_key, &content);
            }

            // Write scene entries to doc
            for (path, godot_scene) in scene_entries {
                // Get the change flag
                let change_type = match tx.get(&files, &path) {
                    Ok(Some(_)) => ChangeType::Modified,
                    _ => ChangeType::Added,
                };

                let scene_file = tx
                    .get_obj_id(&files, &path)
                    .unwrap_or_else(|| tx.put_object(&files, &path, ObjType::Map).unwrap());
                reconcile_prop(&mut tx, &scene_file, "structured_content", godot_scene)
                    .unwrap_or_else(|e| {
                        tracing::error!("error reconciling scene: {}", e);
                        panic!("error reconciling scene: {}", e);
                    });

                changes.push(ChangedFile {
                    path,
                    change_type,
                });
            }

            // Write binary entries to doc
            for (path, binary_doc_handle) in &binary_entries {
                // Get the change flag
                let change_type = match tx.get(&files, path) {
                    Ok(Some(_)) => ChangeType::Modified,
                    _ => ChangeType::Added,
                };

                let file_entry = tx.put_object(&files, path, ObjType::Map);
                let _ = tx.put(
                    file_entry.unwrap(),
                    "url",
                    format!("automerge:{}", &binary_doc_handle.document_id()),
                );

                changes.push(ChangedFile {
                    path: path.clone(),
                    change_type,
                });
            }

            // Handle deleted entries
            for path in deleted_entries {
                let _ = tx.delete(&files, &path);
                changes.push(ChangedFile {
                    path,
                    change_type: ChangeType::Removed,
                });
            }

            // Commit with metadata
            commit_with_attribution_and_timestamp(
                tx,
                &CommitMetadata {
                    username: user_name,
                    branch_id: Some(self.doc_handle.document_id().clone()),
                    merge_metadata: None,
                    reverted_to: match revert {
                        Some(revert) => Some(heads_to_vec_string(revert)),
                        None => None,
                    },
                    changed_files: Some(changes),
                    is_setup: Some(new_project),
                },
            );
        });

        // Return binary doc handles for caller to track
        binary_entries
    }

    /// Get all files from the branch document at specific heads.
    /// 
    /// If `heads` is None, uses the branch's synced_heads.
    /// If `filters` is provided, only returns files matching those paths.
    /// Returns linked document IDs separately since they need to be resolved from doc_handles.
    #[instrument(skip_all, level = tracing::Level::DEBUG)]
    pub fn get_files_at(
        &self,
        heads: Option<&Vec<ChangeHash>>,
        filters: Option<&HashSet<String>>,
    ) -> (HashMap<String, FileContent>, Vec<(DocumentId, String)>) {
        let mut files = HashMap::new();
        let mut linked_doc_ids = Vec::new();

        let heads = match heads {
            Some(heads) => heads.clone(),
            None => self.branch_state.synced_heads.clone(),
        };

        let filtered_paths = if let Some(filters) = filters {
            filters
        } else {
            &HashSet::new()
        };

        self.doc_handle.with_document(|doc| {
            let files_obj_id: ObjId = match doc.get_at(ROOT, "files", &heads) {
                Ok(Some((_, obj_id))) => obj_id,
                Ok(None) => {
                    tracing::warn!("No files object found in branch document");
                    return;
                }
                Err(e) => {
                    tracing::error!("Failed to get files object: {:?}", e);
                    return;
                }
            };

            for path in doc.keys_at(&files_obj_id, &heads) {
                if filtered_paths.len() > 0 && !filtered_paths.contains(&path) {
                    continue;
                }

                let file_entry = match doc.get_at(&files_obj_id, &path, &heads) {
                    Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
                    _ => {
                        tracing::error!("failed to get file entry for {:?}", path);
                        continue;
                    }
                };

                match FileContent::hydrate_content_at(file_entry, doc, &path, &heads) {
                    Ok(content) => {
                        files.insert(path, content);
                    }
                    Err(res) => {
                        match res {
                            Ok(id) => {
                                linked_doc_ids.push((id, path));
                            }
                            Err(error_msg) => {
                                tracing::error!("error: {:?}", error_msg);
                            }
                        }
                    }
                };
            }
        });

        (files, linked_doc_ids)
    }

    /// Get a single file from the branch document at specific heads.
    /// 
    /// returns None if the file doesn't exist.
    /// returns Some(Ok(FileContent)) if the file is found and can be read directly.
    /// returns Some(Err(DocumentId)) if the file is a linked binary document.
    pub fn get_file_at(
        &self,
        path: &str,
        heads: Option<&Vec<ChangeHash>>,
    ) -> Option<Result<FileContent, DocumentId>> {
        let heads = match heads {
            Some(heads) => heads.clone(),
            None => self.branch_state.synced_heads.clone(),
        };

        self.doc_handle.with_document(|doc| {
            let files_obj_id: ObjId = match doc.get_at(ROOT, "files", &heads) {
                Ok(Some((_, obj_id))) => obj_id,
                _ => return None,
            };

            let file_entry = match doc.get_at(&files_obj_id, path, &heads) {
                Ok(Some((automerge::Value::Object(ObjType::Map), file_entry))) => file_entry,
                _ => return None,
            };

            match FileContent::hydrate_content_at(file_entry, doc, &path.to_string(), &heads) {
                Ok(content) => Some(Ok(content)),
                Err(res) => match res {
                    Ok(id) => Some(Err(id)),
                    Err(_) => None,
                },
            }
        })
    }

    /// Get all linked document IDs from this branch.
    /// 
    /// Returns a map of file paths to their linked document IDs.
    pub fn get_linked_docs(&self) -> HashMap<String, DocumentId> {
        get_linked_docs_of_branch(&self.doc_handle)
    }
}

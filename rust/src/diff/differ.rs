use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf, sync::Arc,
};

use automerge::ChangeHash;
use godot::{
    builtin::{GString, Variant}, classes::{ResourceLoader, resource_loader::CacheMode}, global, meta::ToGodot, obj::Singleton
};
use tokio::sync::Mutex;
use tracing::instrument;

use crate::{
    diff::{resource_differ::BinaryResourceDiff, scene_differ::{SceneDiff, TextResourceDiff}, text_differ::TextDiff},
    fs::{file_utils::FileSystemEvent, file_utils::FileContent},
    helpers::{branch::BranchState, utils::ToShortForm},
    interop::godot_accessors::PatchworkEditorAccessor,
    project::{
        branch_db::{BranchDb, HistoryRef},
    },
    project::project::Project,
};

/// The type of change that occurred in a diff.
#[derive(Clone, Debug)]
pub enum ChangeType {
    /// The element was added.
    Added,
    /// The element was modified.
    Modified,
    /// The element was removed.
    Removed,
}

/// A diff for a single file.
#[derive(Clone, Debug)]
pub enum Diff {
    /// A scene file diff.
    Scene(SceneDiff),
    /// A text resource diff.
    TextResourceDiff(TextResourceDiff),
    /// A resource file diff.
    BinaryResource(BinaryResourceDiff),
    /// A text file diff.
    Text(TextDiff),
}

/// A diff for an entire project.
#[derive(Clone, Default, Debug)]
pub struct ProjectDiff {
    /// The file diffs in the project diff.
    pub file_diffs: Vec<Diff>,
}

/// Computes diffs between two sets of heads in a project.
#[derive(Debug)]
pub struct Differ {
    /// Cache that keeps track of the original paths of our loaded ExtResources so far.
    /// original path -> loaded path
    loaded_ext_resources: Arc<Mutex<HashMap<(String, HistoryRef), Arc<Mutex<Option<String>>>>>>,

    /// The [BranchDb] we're working off.
    branch_db: BranchDb,
}

impl Differ {
    /// Creates a new [Differ].
    pub fn new(branch_db: BranchDb) -> Self {
        Self {
            branch_db,
            loaded_ext_resources: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Saves and imports a temp resource at a given path for the specified heads.
    async fn get_resource_at_ref(
        &self,
        path: &String,
        file_content: &FileContent,
        ref_: &HistoryRef,
    ) -> Option<String> {
        let import_path = format!("{}.import", path);
        let mut import_file_content = self.get_file_at_ref(&import_path, ref_).await;
        if import_file_content.is_none() {
            // try at current heads
            if let Some(current_heads) = self.branch_db.get_latest_ref_on_branch(&ref_.branch).await {
                import_file_content = self.get_file_at_ref(&import_path, &current_heads).await;
            }
        }
        return self
            .create_temp_resource_from_content(
                &path,
                &file_content,
                &ref_.heads.first().to_short_form(),
                import_file_content.as_ref(),
            )
            .await;
    }

    /// Creates a temporary resource from file content at a given path.
    pub(super) async fn create_temp_resource_from_content(
        &self,
        path: &str,
        content: &FileContent,
        temp_id: &String,
        import_file_content: Option<&FileContent>,
    ) -> Option<String> {
        let temp_dir = format!("res://.patchwork/temp_{}/", temp_id);
        let temp_path = path.replace("res://", &temp_dir);
        if let Err(e) = content
            .write(&self.branch_db.globalize_path(&temp_path))
            .await
        {
            tracing::error!("error writing file to temp path: {:?}", e);
            return None;
        }

        let mut loaded_path: Option<String> = None;
        if let Some(import_file_content) = import_file_content {
            if let FileContent::String(import_file_content) = import_file_content {
                let import_file_content = import_file_content.replace("res://", &temp_dir);
                // regex to replace uid=uid://<...> and uid=uid://<invalid> with uid=uid://<...> and uid=uid://<invalid>
                let import_file_content =
                    import_file_content.replace(r#"uid=uid://[^\n]+"#, "uid=uid://<invalid>");
                // write the import file content to the temp path
                let import_file_path: String = format!("{}.import", temp_path);
                let _ = FileContent::String(import_file_content).write(&self.branch_db.globalize_path(&import_file_path));

                let loaded_path_str = PatchworkEditorAccessor::import_and_save_resource_to_temp(&temp_path);
                if loaded_path_str.is_empty() {
                    tracing::error!("error importing resource: {:?}", path);
                } else {
                    loaded_path = Some(loaded_path_str);
                }
            }
        } else {
            loaded_path = Some(temp_path.clone());
        }
        // let resource = ResourceLoader::singleton()
        //     .load_ex(&GString::from(&temp_path))
        //     .cache_mode(CacheMode::IGNORE_DEEP)
        //     .done();
        loaded_path
    }

    /// Gets the file content at a given path for the specified heads.
    pub(super) async fn get_file_at_ref(
        &self,
        path: &String,
        ref_: &HistoryRef,
    ) -> Option<FileContent> {
        let mut ret: Option<FileContent> = None;
        {
            let Some(files) = self
                .branch_db
                .get_files_at_ref(ref_, &HashSet::from_iter(vec![path.clone()]))
                .await
            else {
                return None;
            };
            for file in files.into_iter() {
                if file.0 == *path {
                    ret = Some(file.1);
                    break;
                } else {
                    tracing::error!(
                        "Returned a file that didn't match the path!?!??!?!?!?!?!?!!? {:?} != {:?}",
                        file.0,
                        path
                    );
                    return None;
                }
            }
        }
        ret
    }

    /// Loads an ExtResource given a path, using a cache.
    pub(super) async fn start_load_ext_resource(
        &self,
        path: &String,
        ref_: &HistoryRef,
        content: Option<&FileContent>,
    ) -> Option<String> {
        
        let mut hash_map_insert_guard = Some(self.loaded_ext_resources.lock().await);
        if let Some(load_path_tok) = hash_map_insert_guard.as_ref().unwrap().get(&(path.clone(), ref_.clone())).cloned() {
            let load_path_tok = load_path_tok.lock().await;
            if let Some(load_path) = load_path_tok.as_ref().cloned() {
                if ResourceLoader::singleton().load_threaded_request(&load_path) == global::Error::OK {
                    return Some(load_path);
                }
                // else we can't load the path; fall-through to re-create the token and resource
            } else {
                tracing::error!("load path token is None for path: {}", path);
                return None;
            }
        }
        // create the token, acquire the lock so that if this is running on multiple threads it doesn't try and create the resource multiple times
        let mut load_path_tok = Arc::new(Mutex::new(None));
        let mut mutex_guard = load_path_tok.lock().await;
        hash_map_insert_guard.unwrap().insert((path.clone(), ref_.clone()), load_path_tok.clone());
        // release hashmap guard
        hash_map_insert_guard = None;



        let mut resource_content = None;
        if content.is_none() {
            resource_content = self.get_file_at_ref(path, ref_).await;
        }

        let Some(resource_content) = content.or(resource_content.as_ref()) else {
            return None;
        };

        let Some(load_path) = self
            .get_resource_at_ref(path, &content.unwrap_or(&resource_content), ref_)
            .await
        else {
            return None;
        };

        if ResourceLoader::singleton().load_threaded_request(&load_path) == global::Error::OK {
            *mutex_guard = Some(load_path.clone());
            return Some(load_path);
        }
        None
    }

    /// Computes the diff between the two sets of heads.
    #[instrument(skip_all, level = tracing::Level::DEBUG)]
    pub async fn get_diff(&self, before: &HistoryRef, after: &HistoryRef) -> ProjectDiff {
        if before == after {
            tracing::debug!("no changes");
            return ProjectDiff::default();
        }

        // Get the set of new file content that has changed
        let Some(new_file_contents) = self
            .branch_db
            .get_changed_file_content_between_refs(Some(before), after, false)
            .await
        else {
            // Something went wrong
            return ProjectDiff::default();
        };

        let changed_filter: HashSet<String> = new_file_contents
            .iter()
            .map(|event| match event {
                FileSystemEvent::FileCreated(path, _) => path.to_string_lossy().to_string(),
                FileSystemEvent::FileModified(path, _) => path.to_string_lossy().to_string(),
                FileSystemEvent::FileDeleted(path) => path.to_string_lossy().to_string(),
            })
            .collect::<HashSet<String>>();

        // We do need to compare the new files to the old files, so grab the old contents with a filter
        let Some(old_file_contents) = self
            .branch_db
            .get_files_at_ref(before, &changed_filter)
            .await
        else {
            // Something went wrong
            return ProjectDiff::default();
        };

        let mut diffs: Vec<Diff> = vec![];

        for event in &new_file_contents {
            let (path, new_file_content, change_type) = match event {
                FileSystemEvent::FileCreated(path, content) => (path, content, ChangeType::Added),
                FileSystemEvent::FileModified(path, content) => {
                    (path, content, ChangeType::Modified)
                }
                FileSystemEvent::FileDeleted(path) => {
                    (path, &FileContent::Deleted, ChangeType::Removed)
                }
            };
            let path = path.to_string_lossy().to_string();
            let old_file_content = old_file_contents
                .get(&path)
                .unwrap_or(&FileContent::Deleted);

            if matches!(old_file_content, FileContent::Scene(_))
                || matches!(new_file_content, FileContent::Scene(_))
            {
                let old_scene = match old_file_content {
                    FileContent::Scene(s) => Some(s),
                    _ => None,
                };
                let new_scene = match new_file_content {
                    FileContent::Scene(s) => Some(s),
                    _ => None,
                };


                let resource_type = match (old_scene, new_scene) {
                    (None, Some(scene)) => scene.resource_type.clone(),
                    (Some(scene), None) => scene.resource_type.clone(),
                    (_, Some(scene)) => scene.resource_type.clone(),
                    (_, _) => "".to_string(),
                };
                if resource_type == "PackedScene" {
                    diffs.push(Diff::Scene(
                        self.get_scene_diff(&path, old_scene, new_scene, before, after)
                        .await,
                    ));
                } else {
                    diffs.push(Diff::TextResourceDiff(
                        self.get_text_resource_diff(&path, old_scene, new_scene, before, after)
                        .await,
                    ));
                }
            } else if matches!(old_file_content, FileContent::Binary(_))
                || matches!(new_file_content, FileContent::Binary(_))
            {
                // This is a binary file, so use a resource diff
                diffs.push(Diff::BinaryResource(
                    self.get_binary_resource_diff(
                        &path,
                        change_type,
                        old_file_content,
                        new_file_content,
                        before,
                        after,
                    )
                    .await,
                ));
            } else if matches!(old_file_content, FileContent::String(_))
                || matches!(new_file_content, FileContent::String(_))
            {
                // This is a text file, so do a text diff.
                diffs.push(Diff::Text(self.get_text_diff(
                    &path,
                    change_type,
                    old_file_content,
                    new_file_content,
                )));
            } else {
                // We have no idea what type of file this is, so skip it
                continue;
            }
        }

        ProjectDiff { file_diffs: diffs }
    }
}

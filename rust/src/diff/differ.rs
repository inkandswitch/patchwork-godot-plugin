use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use automerge::ChangeHash;
use godot::{
    builtin::{GString, Variant}, classes::{ResourceLoader, resource_loader::CacheMode}, global::str_to_var, meta::ToGodot, obj::Singleton
};
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
pub struct Differ {
    /// Cache that stores our loaded ExtResources so far.
    loaded_ext_resources: RefCell<HashMap<String, Variant>>,

    /// The [BranchDb] we're working off.
    branch_db: BranchDb,
}


/// The different types of Godot-recognized string values that can be stored in a Variant.
#[derive(PartialEq, Debug, Clone)]
pub enum VariantStrValue {
    /// A normal string that doesn't refer to a resource.
    Variant(String),
    /// A Godot resource path string.
    ResourcePath(String),
    /// A Godot sub-resource identifier string.
    SubResourceID(String),
    /// A Godot external resource identifier string (id, path)
    ExtResourceID(String, String),
}


/// Implement the to_string method for this enum
impl std::fmt::Display for VariantStrValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VariantStrValue::Variant(s) => write!(f, "{}", s),
            VariantStrValue::ResourcePath(s) => write!(f, "Resource({})", s),
            VariantStrValue::SubResourceID(s) => write!(f, "SubResource({})", s),
            VariantStrValue::ExtResourceID(id, _path) => write!(f, "ExtResource({})", id),
        }
    }
}

pub trait ContentLoader {
    fn get_branch_db(&self) -> &BranchDb;
    fn get_loaded_ext_resources(&self) -> &RefCell<HashMap<String, Variant>>;
    
    async fn get_resource_at_ref(
        &self,
        path: &String,
        file_content: &FileContent,
        ref_: &HistoryRef,
    ) -> Option<Variant> {
        let import_path = format!("{}.import", path);
        let mut import_file_content = self.get_file_at_ref(&import_path, ref_).await;
        // TODO (Lilith): Reimplement this using branchDB
        // if import_file_content.is_none() {
        //     // try at current heads
        //     import_file_content = self.get_file_at_ref(&import_path, None);
        // }
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
    async fn create_temp_resource_from_content(
        &self,
        path: &str,
        content: &FileContent,
        temp_id: &String,
        import_file_content: Option<&FileContent>,
    ) -> Option<Variant> {
        let temp_dir = format!("res://.patchwork/temp_{}/", temp_id);
        let temp_path = path.replace("res://", &temp_dir);
        if let Err(e) = content
            .write(&self.get_branch_db().globalize_path(&temp_path))
            .await
        {
            tracing::error!("error writing file to temp path: {:?}", e);
            return None;
        }

        if let Some(import_file_content) = import_file_content {
            if let FileContent::String(import_file_content) = import_file_content {
                let import_file_content = import_file_content.replace("res://", &temp_dir);
                // regex to replace uid=uid://<...> and uid=uid://<invalid> with uid=uid://<...> and uid=uid://<invalid>
                let import_file_content =
                    import_file_content.replace(r#"uid=uid://[^\n]+"#, "uid=uid://<invalid>");
                // write the import file content to the temp path
                let import_file_path: String = format!("{}.import", temp_path);
                let _ = FileContent::String(import_file_content).write(&self.get_branch_db().globalize_path(&import_file_path));

                let res = PatchworkEditorAccessor::import_and_load_resource(&temp_path);
                if res.is_nil() {
                    tracing::error!("error importing resource: {:?}", temp_path);
                    return None;
                }
                tracing::debug!("successfully imported resource: {:?}", temp_path);
                return Some(res);
            }
        }
        let resource = ResourceLoader::singleton()
            .load_ex(&GString::from(&temp_path))
            .cache_mode(CacheMode::IGNORE_DEEP)
            .done();
        if let Some(resource) = resource {
            return Some(resource.to_variant());
        }
        None
    }

    /// Gets the file content at a given path for the specified heads.
    async fn get_file_at_ref(
        &self,
        path: &String,
        ref_: &HistoryRef,
    ) -> Option<FileContent> {
        let mut ret: Option<FileContent> = None;
        {
            let Some(files) = self
                .get_branch_db()
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
    async fn load_ext_resource(
        &self,
        path: &String,
        ref_: &HistoryRef,
    ) -> Option<Variant> {
        if let Some(resource) = self.get_loaded_ext_resources().borrow().get(path) {
            return Some(resource.clone());
        }

        let resource_content = self.get_file_at_ref(path, ref_).await;
        let Some(resource_content) = resource_content else {
            return None;
        };

        let Some(resource) = self
            .get_resource_at_ref(path, &resource_content, ref_)
            .await
        else {
            return None;
        };

        self.get_loaded_ext_resources()
            .borrow_mut()
            .insert(path.clone(), resource.clone());
        Some(resource)
    }

    /// Returns the value of a given prop, within a given scene.
    /// Normally, it's a String. If it's a (non-script) ExtResource or ResourcePath,
    /// it loads and returns the resource content as a Variant.
    async fn get_prop_value(
        &self,
        prop_value: &VariantStrValue,
        is_script: bool,
        hist_ref: &HistoryRef,
    ) -> Variant {
        // Prevent loading script files during the diff and creating issues for the editor
        if is_script {
            return "<Script changed>".to_variant();
        }
        let path;
        match prop_value {
            VariantStrValue::Variant(variant) => {
                return str_to_var(variant);
            }
            VariantStrValue::SubResourceID(sub_resource_id) => {
                // We currently don't support displaying deep subresource diffs, so just inform of a change.
                return format!("<SubResource {} changed>", sub_resource_id).to_variant();
            }
            VariantStrValue::ResourcePath(resource_path) => {
                path = resource_path;
            }
            VariantStrValue::ExtResourceID(ext_resource_id, _path) => {
                path = _path;
            }
        }

        let Some(resource) = self.load_ext_resource(&path, hist_ref).await
        else {
            return "<ExtResource load failed>".to_variant();
        };

        return resource;
    }

    async fn get_resource(
        &self,
        path: &String,
        content: &FileContent,
        ref_: &HistoryRef,
    ) -> Option<Variant> {
        let import_path = format!("{}.import", path);
        let import_file_content = match content {
            FileContent::Deleted => None,
            _ => self
                .get_file_at_ref(&import_path, ref_).await
                // TODO (Lilith): make this work
                // try at current heads 
                // .or(self.get_file_at(&import_path, None)),
        };

        self.create_temp_resource_from_content(
            &path,
            content,
            &ref_.heads.first().to_short_form(),
            import_file_content.as_ref(),
        ).await
    }

}


impl ContentLoader for Differ {
    fn get_branch_db(&self) -> &BranchDb {
        &self.branch_db
    }
    fn get_loaded_ext_resources(&self) -> &RefCell<HashMap<String, Variant>> {
        &self.loaded_ext_resources
    }
}


impl Differ {
    /// Creates a new [Differ].
    pub fn new(branch_db: BranchDb) -> Self {
        Self {
            branch_db,
            loaded_ext_resources: RefCell::new(HashMap::new()),
        }
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

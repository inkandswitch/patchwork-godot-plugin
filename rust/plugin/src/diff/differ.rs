use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use automerge::{ChangeHash};
use godot::{
    builtin::{GString, Variant, VariantType},
    classes::{ResourceLoader, resource_loader::CacheMode},
    meta::ToGodot,
};
use tracing::instrument;

use crate::{
    diff::{
        resource_differ::ResourceDiff, scene_differ::SceneDiff,
        text_differ::TextDiff,
    },
    fs::{file_system_driver::FileSystemEvent, file_utils::FileContent},
    helpers::{
        branch::BranchState,
        utils::{ToShortForm},
    },
    interop::{godot_accessors::PatchworkEditorAccessor, godot_helpers::VariantTypeGetter},
    project::project::Project,
};

#[derive(Clone, Debug)]
pub enum ChangeType {
    Added,
    Modified,
    Removed,
}

#[derive(Clone, Debug)]
pub enum Diff {
    Scene(SceneDiff),
    Resource(ResourceDiff),
    Text(TextDiff),
}

#[derive(Clone, Default, Debug)]
pub struct ProjectDiff {
    pub file_diffs: Vec<Diff>,
}

pub struct Differ<'a> {
    pub(super) project: &'a Project,

    /// The heads we're currently diffing between.
    pub(super) curr_heads: Vec<ChangeHash>,
    pub(super) prev_heads: Vec<ChangeHash>,

    /// Cache that stores our loaded ExtResources so far.
    loaded_ext_resources: RefCell<HashMap<String, Variant>>,

    // The branch we're currently diffing on
    pub(super) branch_state: &'a BranchState,
}

impl<'a> Differ<'a> {
	pub fn new(
		project: &'a Project,
		curr_heads: Vec<ChangeHash>,
		prev_heads: Vec<ChangeHash>,
		branch_state: &'a BranchState,
	) -> Self {

        let curr_heads = if curr_heads.len() == 0 {
            branch_state.synced_heads.clone()
        } else {
            curr_heads
        };

		Self {
			project,
			curr_heads,
			prev_heads,
			loaded_ext_resources: RefCell::new(HashMap::new()),
			branch_state,
		}
	}

    fn get_resource_at(
        &self,
        path: &String,
        file_content: &FileContent,
        heads: &Vec<ChangeHash>,
    ) -> Option<Variant> {
        let import_path = format!("{}.import", path);
        let mut import_file_content = self.get_file_at(&import_path, Some(heads));
        if import_file_content.is_none() {
            // try at current heads
            import_file_content = self.get_file_at(&import_path, None);
        }
        return self.create_temp_resource_from_content(
            &path,
            &file_content,
            &heads,
            import_file_content.as_ref(),
        );
    }

    pub(super) fn create_temp_resource_from_content(
        &self,
        path: &str,
        content: &FileContent,
        heads: &Vec<ChangeHash>,
        import_file_content: Option<&FileContent>,
    ) -> Option<Variant> {
        let temp_dir = format!("res://.patchwork/temp_{}/", heads.first().to_short_form());
        let temp_path = path.replace("res://", &temp_dir);
        if let Err(e) = FileContent::write_file_content(
            &PathBuf::from(self.project.globalize_path(&temp_path)),
            content,
        ) {
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
                let _ = FileContent::write_file_content(
                    &PathBuf::from(self.project.globalize_path(&import_file_path)),
                    &FileContent::String(import_file_content),
                );

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
            .load_ex(&GString::from(temp_path))
            .cache_mode(CacheMode::IGNORE_DEEP)
            .done();
        if let Some(resource) = resource {
            return Some(resource.to_variant());
        }
        None
    }

    pub(super) fn get_file_at(
        &self,
        path: &String,
        heads: Option<&Vec<ChangeHash>>,
    ) -> Option<FileContent> {
        let mut ret: Option<FileContent> = None;
        {
            let files = self
                .project
                .get_files_at(heads, Some(&HashSet::from_iter(vec![path.clone()])));
            for file in files.into_iter() {
                if file.0 == *path {
                    ret = Some(file.1);
                    break;
                } else {
                    panic!(
                        "Returned a file that didn't match the path!?!??!?!?!?!?!?!!? {:?} != {:?}",
                        file.0, path
                    );
                }
            }
        }
        ret
    }

    /// Loads an ExtResource given a path, using a cache.
    pub(super) fn load_ext_resource(
        &self,
        path: &String,
        heads: &Vec<ChangeHash>,
    ) -> Option<Variant> {
        if let Some(resource) = self.loaded_ext_resources.borrow().get(path) {
            return Some(resource.clone());
        }

        let resource_content = self.get_file_at(path, Some(heads));
        let Some(resource_content) = resource_content else {
            return None;
        };

        let Some(resource) = self.get_resource_at(path, &resource_content, heads) else {
            return None;
        };

        self
            .loaded_ext_resources
            .borrow_mut()
            .insert(path.clone(), resource.clone());
		Some(resource)
    }

    #[instrument(skip_all, level = tracing::Level::DEBUG)]
    pub fn get_diff(&self) -> ProjectDiff {
        tracing::debug!(
            "branch {:?}, getting changes between {} and {}",
            self.branch_state.name,
            self.prev_heads.to_short_form(),
            self.curr_heads.to_short_form()
        );

        if self.prev_heads == self.curr_heads {
            tracing::debug!("no changes");
            return ProjectDiff::default();
        }

        let mut diffs: Vec<Diff> = vec![];
        // Get old and new content
        let new_file_contents = self.project.get_changed_file_content_between(
            None,
            self.branch_state.doc_handle.document_id().clone(),
            self.prev_heads.clone(),
            self.curr_heads.clone(),
            false,
        );
        let changed_files_set: HashSet<String> = new_file_contents
            .iter()
            .map(|event| match event {
                FileSystemEvent::FileCreated(path, _) => path.to_string_lossy().to_string(),
                FileSystemEvent::FileModified(path, _) => path.to_string_lossy().to_string(),
                FileSystemEvent::FileDeleted(path) => path.to_string_lossy().to_string(),
            })
            .collect::<HashSet<String>>();
        let old_file_contents = self.project.get_files_on_branch_at(
            self.branch_state,
            Some(&self.prev_heads),
            Some(&changed_files_set),
        );

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
            let old_content_type = old_file_content.get_variant_type();
            let new_content_type = new_file_content.get_variant_type();
            let old_content = match old_content_type {
                VariantType::NIL => None,
                _ => Some(old_file_content),
            };
            let new_content = match new_content_type {
                VariantType::NIL => None,
                _ => Some(new_file_content),
            };

            if old_content_type == VariantType::OBJECT || new_content_type == VariantType::OBJECT {
                // This is a scene file, so use a scene diff
                diffs.push(Diff::Scene(self.get_scene_diff(&path)));
            } else if old_content_type != VariantType::STRING
                && new_content_type != VariantType::STRING
            {
                // This is a binary file, so use a resource diff
                diffs.push(Diff::Resource(self.get_resource_diff(
                    &path,
                    change_type,
                    old_content,
                    new_content
                )));
            } else if old_content_type != VariantType::PACKED_BYTE_ARRAY
                && new_content_type != VariantType::PACKED_BYTE_ARRAY
            {
                // This is a text file, so do a text diff.
                diffs.push(Diff::Text(self.get_text_diff(
                    &path,
                    change_type,
                    old_content,
                    new_content,
                )));
            } else {
                // We have no idea what type of file this is, so skip it
				continue;
            }
        }

        ProjectDiff { file_diffs: diffs }
    }
}

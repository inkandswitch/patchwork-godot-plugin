use std::{
    cell::RefCell, collections::{HashMap, HashSet}, path::PathBuf, str::FromStr, sync::Arc
};

use automerge::ChangeHash;
use godot::{
    builtin::{GString, Variant}, classes::{ResourceLoader, resource_loader::CacheMode}, global, meta::ToGodot, obj::Singleton
};
use tokio::sync::Mutex;
use tracing::instrument;

use crate::{
    diff::{resource_differ::BinaryResourceDiff, scene_differ::{SceneDiff, TextResourceDiff}, text_differ::TextDiff},
    fs::file_utils::{FileContent, FileSystemEvent},
    helpers::{branch::BranchState, utils::ToShortForm},
    interop::godot_accessors::PatchworkEditorAccessor,
    project::{branch_db::{BranchDb, HistoryRef, HistoryRefPath}, project::Project},
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
    /// The [BranchDb] we're working off.
    branch_db: BranchDb,
}

impl Differ {
    /// Creates a new [Differ].
    pub fn new(branch_db: BranchDb) -> Self {
        Self {
            branch_db,
        }
    }

    /// Loads an ExtResource given a path, using a cache.
    pub(super) async fn start_load_ext_resource(
        &self,
        path: &String,
        ref_: &HistoryRef
    ) -> Option<String> {
        let history_ref_path = HistoryRefPath::make_path_string(ref_, path).ok()?;

        if ResourceLoader::singleton().load_threaded_request(&history_ref_path) == global::Error::OK {
            return Some(history_ref_path);
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

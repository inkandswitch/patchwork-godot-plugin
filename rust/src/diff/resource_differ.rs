use godot::builtin::Variant;

use crate::{
    diff::differ::{ChangeType, ContentLoader, Differ}, fs::file_utils::FileContent, helpers::utils::ToShortForm, project::branch_db::HistoryRef
};


#[derive(Clone, Debug)]
pub struct BinaryResourceDiff {
    pub path: String,
    pub change_type: ChangeType,
    pub old_content: Option<FileContent>,
    pub new_content: Option<FileContent>,
    pub before: HistoryRef,
    pub after: HistoryRef,
}

impl BinaryResourceDiff {
    pub fn new(
        path: String,
        change_type: ChangeType,
        old_content: Option<FileContent>,
        new_content: Option<FileContent>,
        before: HistoryRef,
        after: HistoryRef,
    ) -> BinaryResourceDiff {
        BinaryResourceDiff {
            path,
            change_type,
            old_content,
            new_content,
            before,
            after,
        }
    }
}

impl Differ {
    pub(super) async fn get_binary_resource_diff(
        &self,
        path: &String,
        change_type: ChangeType,
        old_content: &FileContent,
        new_content: &FileContent,
        before: &HistoryRef,
        after: &HistoryRef
    ) -> BinaryResourceDiff {
        let old_content = if matches!(old_content, FileContent::Deleted) { None } else { Some(old_content.clone()) };
        let new_content = if matches!(new_content, FileContent::Deleted) { None } else { Some(new_content.clone()) };
        BinaryResourceDiff::new(
            path.clone(),
            change_type,
            old_content,
            new_content,
            before.clone(),
            after.clone(),
        )
    }

}

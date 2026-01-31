use godot::builtin::Variant;

use crate::{
    diff::{differ::{ChangeType, Differ}, scene_differ::VariantValue}, fs::file_utils::FileContent, helpers::utils::ToShortForm, project::branch_db::HistoryRef
};


#[derive(Clone, Debug)]
pub struct BinaryResourceDiff {
    pub path: String,
    pub change_type: ChangeType,
    pub old_resource: Option<VariantValue>,
    pub new_resource: Option<VariantValue>,
}

impl BinaryResourceDiff {
    pub fn new(
        path: String,
        change_type: ChangeType,
        old_resource: Option<VariantValue>,
        new_resource: Option<VariantValue>,
    ) -> BinaryResourceDiff {
        BinaryResourceDiff {
            path,
            change_type,
            old_resource,
            new_resource,
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
        BinaryResourceDiff::new(
            path.clone(),
            change_type,
            self.get_resource(path, old_content, before).await,
            self.get_resource(path, new_content, after).await,
        )
    }

    async fn get_resource(
        &self,
        path: &String,
        _content: &FileContent,
        ref_: &HistoryRef,
    ) -> Option<VariantValue> {

        let Some(load_path) = self.start_load_ext_resource(path, ref_).await
        else {
            return None;
        };
        Some(VariantValue::LazyLoadData(path.clone(), load_path))
    }
}

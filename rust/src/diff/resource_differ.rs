use godot::builtin::Variant;

use crate::{
    diff::{differ::{ChangeType, Differ}, scene_differ::DiffVariantValue}, fs::file_utils::FileContent, helpers::utils::ToShortForm, parser::parser_defs::VariantVal, project::branch_db::HistoryRef
};


#[derive(Clone, Debug)]
pub struct BinaryResourceDiff {
    pub path: String,
    pub change_type: ChangeType,
    pub old_resource: Option<DiffVariantValue>,
    pub new_resource: Option<DiffVariantValue>,
}

impl BinaryResourceDiff {
    pub fn new(
        path: String,
        change_type: ChangeType,
        old_resource: Option<DiffVariantValue>,
        new_resource: Option<DiffVariantValue>,
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
    ) -> Option<DiffVariantValue> {
        if matches!(_content, FileContent::Deleted) {
            return None;
        }

        match self.start_load_ext_resource(&path, ref_).await{
            Ok(load_path) => Some(DiffVariantValue::LazyLoadData(path.clone(), load_path)),
            Err(e) => Some(DiffVariantValue::Variant(VariantVal::String(format!("\"<ExtResource {} load failed ({})>\"", path, e)))),
        }

    }
}

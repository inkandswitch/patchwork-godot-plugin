use godot::builtin::Variant;

use crate::{
    diff::differ::{ChangeType, Differ}, fs::file_utils::FileContent, helpers::utils::ToShortForm, project::branch_db::HistoryRef
};

#[derive(Clone, Debug)]
pub struct ResourceDiff {
    pub path: String,
    pub change_type: ChangeType,
    pub old_resource: Option<Variant>,
    pub new_resource: Option<Variant>,
}

impl ResourceDiff {
    pub fn new(
        path: String,
        change_type: ChangeType,
        old_resource: Option<Variant>,
        new_resource: Option<Variant>,
    ) -> ResourceDiff {
        ResourceDiff {
            path,
            change_type,
            old_resource,
            new_resource,
        }
    }
}

impl Differ {
    pub(super) async fn get_resource_diff(
        &self,
        path: &String,
        change_type: ChangeType,
        old_content: &FileContent,
        new_content: &FileContent,
        before: &HistoryRef,
        after: &HistoryRef
    ) -> ResourceDiff {
        ResourceDiff::new(
            path.clone(),
            change_type,
            self.get_resource(path, old_content, before).await,
            self.get_resource(path, new_content, after).await,
        )
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

use automerge::ChangeHash;
use godot::builtin::Variant;

use crate::{
    diff::differ::{ChangeType, Differ},
    fs::file_utils::FileContent,
};


#[derive(Clone, Debug)]
pub struct BinaryResourceDiff {
    pub path: String,
    pub change_type: ChangeType,
    pub old_resource: Option<Variant>,
    pub new_resource: Option<Variant>,
}

impl BinaryResourceDiff {
    pub fn new(
        path: String,
        change_type: ChangeType,
        old_resource: Option<Variant>,
        new_resource: Option<Variant>,
    ) -> BinaryResourceDiff {
        BinaryResourceDiff {
            path,
            change_type,
            old_resource,
            new_resource,
        }
    }
}

impl Differ<'_> {
    pub(super) fn get_binary_resource_diff(
        &self,
        path: &String,
        change_type: ChangeType,
        old_content: &FileContent,
        new_content: &FileContent,
    ) -> BinaryResourceDiff {
        BinaryResourceDiff::new(
            path.clone(),
            change_type,
            self.get_resource(path, old_content, &self.prev_heads),
            self.get_resource(path, new_content, &self.curr_heads),
        )
    }

    fn get_resource(
        &self,
        path: &String,
        content: &FileContent,
        heads: &Vec<ChangeHash>,
    ) -> Option<Variant> {
        let import_path = format!("{}.import", path);
        let import_file_content = match content {
            FileContent::Deleted => None,
            _ => self
                .get_file_at(&import_path, Some(heads))
                // try at current heads
                .or(self.get_file_at(&import_path, None)),
        };

        self.create_temp_resource_from_content(
            &path,
            content,
            &self.prev_heads,
            import_file_content.as_ref(),
        )
    }
}

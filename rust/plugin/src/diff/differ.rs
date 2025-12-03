
use std::collections::{HashMap, HashSet};

use automerge::ChangeHash;
use godot::builtin::Variant;

use crate::fs::file_utils::FileContent;


pub struct TextDiffLine {
	pub new_line_no: i64,
	pub old_line_no: i64,
	pub content: String,
	pub status: String,
}

pub struct TextDiffHunk {
    pub new_start: i64,
    pub old_start: i64,
    pub new_lines: i64,
    pub old_lines: i64,
    pub diff_lines: Vec<TextDiffLine>,
}

pub struct TextDiff {
	pub path: String,
	pub diff_hunks: Vec<TextDiffHunk>,
	pub change_type: ChangeType
}


impl TextDiff {
	pub fn create(
		path: &String,
		old_text: &String,
		new_text: &String,
		change_type: ChangeType
	) -> TextDiff {
		let diff = similar::TextDiff::from_lines(old_text, new_text);
		let mut unified = diff.unified_diff();
		unified.header(path, path);

		fn get_range(ops: &[similar::DiffOp]) -> (usize, usize, usize, usize) {
			let first = ops[0];
			let last = ops[ops.len() - 1];
			let old_start = first.old_range().start;
			let new_start = first.new_range().start;
			let old_end = last.old_range().end;
			let new_end = last.new_range().end;
			(
				old_start + 1,
				new_start + 1,
				old_end - old_start,
				new_end - new_start,
			)
		}
		let mut diff_file = TextDiff {
			path: path.clone(),
			diff_hunks: Vec::new(),
			change_type
		};
		for (_i, hunk) in unified.iter_hunks().enumerate() {
			let (old_start, new_start, old_lines, new_lines) = get_range(&hunk.ops());
			let mut diff_hunk = TextDiffHunk {
				old_start: old_start as i64,
				new_start: new_start as i64,
				old_lines: old_lines as i64,
				new_lines: new_lines as i64,
				diff_lines: Vec::new(),
			};
			for (_idx, change) in hunk.iter_changes().enumerate() {
				let diff_line = TextDiffLine {
					new_line_no: if let Some(new_index) = change.new_index() {
						new_index as i64 + 1
					} else {
						-1
					},
					old_line_no: if let Some(old_index) = change.old_index() {
						old_index as i64 + 1
					} else {
						-1
					},
					content: change.as_str().unwrap().to_string(),
					status: match change.tag() {
						similar::ChangeTag::Equal => " ",
						similar::ChangeTag::Delete => "-",
						similar::ChangeTag::Insert => "+",
					}.to_string()
				};
				diff_hunk.diff_lines.push(diff_line);
			}
			diff_file.diff_hunks.push(diff_hunk);
		}
		diff_file
	}
}


pub struct ResourceDiff {
	pub change_type: ChangeType,
	pub old_heads: Vec<ChangeHash>,
	pub new_heads: Vec<ChangeHash>,
	pub old_content: Option<FileContent>,
    pub new_content: Option<FileContent>,
    pub old_import_info: Option<FileContent>,
    pub new_import_info: Option<FileContent>,
}
		// need this for interop
        // if let Some(old_content) = imported_diff.old_content.as_ref() {
        //     if let Some(old_resource) = self.create_temp_resource_from_content(
        //         &path,
        //         old_content,
        //         &imported_diff.old_heads,
        //         imported_diff.old_import_info.as_ref(),
        //     ) {
        //         let _ = result.insert("old_resource", old_resource);
        //     }
        // }
        // if let Some(new_content) = imported_diff.new_content.as_ref() {
        //     if let Some(new_resource) = self.create_temp_resource_from_content(
        //         &path,
        //         new_content,
        //         &imported_diff.new_heads,
        //         imported_diff.new_import_info.as_ref(),
        //     ) {
        //         let _ = result.insert("new_resource", new_resource);
        //     }
        // }
impl ResourceDiff {
	pub fn new(
		change_type: ChangeType,
		old_heads: Vec<ChangeHash>,
		new_heads: Vec<ChangeHash>,
		old_content: Option<FileContent>,
		new_content: Option<FileContent>,
		old_import_info: Option<FileContent>,
		new_import_info: Option<FileContent>,
	) -> ResourceDiff {
		ResourceDiff {
			change_type,
			old_heads,
			new_heads,
			old_content,
			new_content,
			old_import_info,
			new_import_info,
		}
	}
}

#[derive(Clone)]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone)]
pub struct PropertyDiff {
    name: String,
    change_type: ChangeType,
    old_value: Option<Variant>,
    new_value: Option<Variant>,
}

impl PropertyDiff {
    pub fn new(
        name: String,
        change_type: ChangeType,
        old_value: Option<Variant>,
        new_value: Option<Variant>,
    ) -> Self {
        PropertyDiff {
            name,
            change_type,
            old_value,
            new_value,
        }
    }
}

pub struct NodeDiff {
	change_type: ChangeType,
	node_path: String,
	node_type: String,
	changed_properties: HashMap<String, PropertyDiff>
}

impl NodeDiff {
	pub fn new(change_type: ChangeType, node_path: String, node_type: String, changed_properties: HashMap<String, PropertyDiff>) -> NodeDiff {
		NodeDiff {
			change_type,
			node_path,
			node_type,
			changed_properties
		}
	}
}

pub struct SceneDiff {
	path: String,
	changed_nodes: Vec<NodeDiff>,
}
impl SceneDiff {
	pub fn new(path: String, changed_nodes: Vec<NodeDiff>) -> SceneDiff {
		SceneDiff {
			path,
			changed_nodes,
		}
	}

}

pub struct FileDiff {
	pub path: String,
	pub change_type: ChangeType
}

impl FileDiff {
	pub fn new(path: &String, change_type: ChangeType) -> FileDiff {
		FileDiff {
			path: path.clone(),
			change_type
		}
	}
}

pub enum Diff {
	File(FileDiff),
	Scene(SceneDiff),
	Resource(ResourceDiff),
	Text(TextDiff)
}

#[derive(Default)]
pub struct ProjectDiff {
	pub file_diffs: Vec<Diff>
}

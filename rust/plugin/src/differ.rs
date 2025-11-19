
use automerge::ChangeHash;
use crate::file_utils::FileContent;



pub struct DiffLine {
	pub new_line_no: i64,
	pub old_line_no: i64,
	pub content: String,
	pub status: String,
// These are manipulated by the diff viewer, no need to include them
//     String old_text;
//     String new_text;
}

pub struct DiffHunk {
    pub new_start: i64,
    pub old_start: i64,
    pub new_lines: i64,
    pub old_lines: i64,
    pub diff_lines: Vec<DiffLine>,
}

pub struct TextDiffFile {
	pub new_file: String,
	pub old_file: String,
	pub diff_hunks: Vec<DiffHunk>,
}


impl TextDiffFile {
	pub fn create(
		old_path: String,
		new_path: String,
		old_text: &String,
		new_text: &String,
	) -> TextDiffFile {
		let diff = similar::TextDiff::from_lines(old_text, new_text);
		let mut unified = diff.unified_diff();
		unified.header(old_path.as_str(), new_path.as_str());

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
		let mut diff_file = TextDiffFile {
			new_file: new_path,
			old_file: old_path,
			diff_hunks: Vec::new(),
		};
		for (_i, hunk) in unified.iter_hunks().enumerate() {
			let header = hunk.header();
			let (old_start, new_start, old_lines, new_lines) = get_range(&hunk.ops());
			let mut diff_hunk = DiffHunk {
				old_start: old_start as i64,
				new_start: new_start as i64,
				old_lines: old_lines as i64,
				new_lines: new_lines as i64,
				diff_lines: Vec::new(),
			};
			for (_idx, change) in hunk.iter_changes().enumerate() {
				let diff_line = DiffLine {
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


pub struct ImportedDiff {
	pub old_heads: Vec<ChangeHash>,
	pub new_heads: Vec<ChangeHash>,
	pub old_content: Option<FileContent>,
    pub new_content: Option<FileContent>,
    pub old_import_info: Option<FileContent>,
    pub new_import_info: Option<FileContent>,
}

impl ImportedDiff {
	pub fn create(
		old_heads: Vec<ChangeHash>,
		new_heads: Vec<ChangeHash>,
		old_content: Option<FileContent>,
		new_content: Option<FileContent>,
		old_import_info: Option<FileContent>,
		new_import_info: Option<FileContent>,
	) -> ImportedDiff {
		ImportedDiff {
			old_heads,
			new_heads,
			old_content,
			new_content,
			old_import_info,
			new_import_info,
		}
	}
}

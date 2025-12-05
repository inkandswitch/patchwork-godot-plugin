use crate::{
    diff::differ::{ChangeType, Differ},
    fs::file_utils::FileContent,
};

#[derive(Clone, Debug)]
pub struct TextDiffLine {
    pub new_line_no: i64,
    pub old_line_no: i64,
    pub content: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct TextDiffHunk {
    pub new_start: i64,
    pub old_start: i64,
    pub new_lines: i64,
    pub old_lines: i64,
    pub diff_lines: Vec<TextDiffLine>,
}

#[derive(Clone, Debug)]
pub struct TextDiff {
    pub path: String,
    pub diff_hunks: Vec<TextDiffHunk>,
    pub change_type: ChangeType,
}

impl TextDiff {
    fn create(
        path: &String,
        old_text: &String,
        new_text: &String,
        change_type: ChangeType,
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
            change_type,
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
                    }
                    .to_string(),
                };
                diff_hunk.diff_lines.push(diff_line);
            }
            diff_file.diff_hunks.push(diff_hunk);
        }
        diff_file
    }
}

impl Differ<'_> {
    pub(super) fn get_text_diff(
        &self,
        path: &String,
        change_type: ChangeType,
        old_content: &FileContent,
        new_content: &FileContent,
    ) -> TextDiff {
        let empty_string = String::from("");
        let old_text = if let FileContent::String(s) = old_content {
            &s
        } else {
            &empty_string
        };
        let new_text = if let FileContent::String(s) = new_content {
            &s
        } else {
            &empty_string
        };
        TextDiff::create(path, old_text, new_text, change_type)
    }
}

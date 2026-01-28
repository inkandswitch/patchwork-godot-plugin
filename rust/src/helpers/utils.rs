use std::{
    collections::{HashMap, HashSet}, fmt, path::Path, str::FromStr, time::{SystemTime, UNIX_EPOCH}
};

use crate::{diff::differ::ProjectDiff, helpers::{branch::BranchState, doc_utils::SimpleDocReader}, project::branch_db::HistoryRef};
use automerge::{
    Automerge, Change, ChangeHash, Patch, PatchLog, ROOT, ReadDoc, transaction::{CommitOptions, Transaction}
};
use samod::{DocHandle, DocumentId};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[inline(always)]
pub(crate) fn get_automerge_doc_diff(doc: &Automerge, old_heads: &[ChangeHash], new_heads: &[ChangeHash]) -> Vec<Patch> {
	#[cfg(not(feature = "automerge_0_6"))]
	{
		doc.diff(old_heads, new_heads)
	}
	#[cfg(feature = "automerge_0_6")]
	{
		doc.diff(old_heads, new_heads, automerge::patches::TextRepresentation::String(automerge::TextEncoding::Utf8CodeUnit))
	}
}


pub(crate) fn get_changed_files(patches: &Vec<automerge::Patch>) -> HashSet<String> {
    let mut changed_files = HashSet::new();

    // log all patches
    for patch in patches.iter() {
        let first_key = match patch.path.get(0) {
            Some((_, prop)) => match prop {
                automerge::Prop::Map(string) => string,
                _ => continue,
            },
            _ => continue,
        };

        // get second key
        let second_key = match patch.path.get(1) {
            Some((_, prop)) => match prop {
                automerge::Prop::Map(string) => string,
                _ => continue,
            },
            _ => continue,
        };

        if first_key == "files" {
            changed_files.insert(second_key.to_string());
        }

        // tracing::debug!("changed files: {:?}", changed_files);
    }

    return changed_files;
}

pub(crate) fn parse_automerge_url(url: &str) -> Option<DocumentId> {
    const PREFIX: &str = "automerge:";
    if !url.starts_with(PREFIX) {
        return None;
    }

    let hash = &url[PREFIX.len()..];
    DocumentId::from_str(hash).ok()
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct MergeMetadata {
    pub merged_branch_id: DocumentId,
    pub merged_at_heads: Vec<ChangeHash>,
    pub forked_at_heads: Vec<ChangeHash>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum ChangeType {
	Added,
	Removed,
	Modified
}

impl fmt::Display for ChangeType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match self {
			ChangeType::Added => write!(f, "added"),
			ChangeType::Removed => write!(f, "removed"),
			ChangeType::Modified => write!(f, "modified")
		}
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ChangedFile {
    pub change_type: ChangeType,
    pub path: String
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CommitMetadata {
    pub username: Option<String>,
    pub branch_id: Option<DocumentId>,
    pub merge_metadata: Option<MergeMetadata>,
	pub reverted_to: Option<Vec<String>>,
    /// Changed files in this commit. Only valid for commits to branch documents.
    pub changed_files: Option<Vec<ChangedFile>>,
	/// Whether this change was created to initialize the repository.
	pub is_setup: Option<bool>
}

pub(crate) fn commit_with_metadata(tx: Transaction, metadata: &CommitMetadata) {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let message = serde_json::json!(metadata).to_string();

    tx.commit_with(
        CommitOptions::default()
            .with_message(message)
            .with_time(timestamp),
    );
}

pub(crate) fn print_branch_state(message: &str, branch_state: &BranchState) {
	let last_synced_head = branch_state.synced_heads.last().map(|h| h.to_short_form()).unwrap_or("<NONE>".to_string());
    tracing::info!(
        "{}: {:?} - linked docs: {:?}, last synced head: {:?}",
        &message, branch_state.name, branch_state.linked_doc_ids.len(), last_synced_head
    );
	tracing::debug!("branch id: {:?}", branch_state.doc_handle.document_id());
	tracing::trace!("linked doc ids: {:?}", branch_state.linked_doc_ids);
	tracing::trace!("synced heads: {:?}", branch_state.synced_heads);
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitInfo {
	pub hash: ChangeHash,
	pub timestamp: i64,
	pub metadata: Option<CommitMetadata>,
    pub synced: bool,
	pub summary: String
}

#[derive(Debug)]
pub struct BranchWrapper {
	pub state: BranchState,
	pub children: Vec<DocumentId>
}

#[derive(Debug)]
pub struct DiffWrapper {
	pub diff: ProjectDiff,
	pub title: String
}

pub(crate) fn heads_to_vec_string(heads: Vec<ChangeHash>) -> Vec<String> {
    heads
        .iter()
        .map(|h| h.to_string())
        .collect()
}


pub trait ToShortForm {
    fn to_short_form(&self) -> String;
}

impl ToShortForm for ChangeHash {
    fn to_short_form(&self) -> String {
        self.to_string().chars().take(7).collect::<String>()
    }
}

impl ToShortForm for Option<&ChangeHash> {
    fn to_short_form(&self) -> String {
        match self {
            Some(change_hash) => change_hash.to_short_form(),
            None => "<NONE>".to_string(),
        }
    }
}

impl ToShortForm for Option<ChangeHash> {
    fn to_short_form(&self) -> String {
        match self {
            Some(change_hash) => change_hash.to_short_form(),
            None => "<NONE>".to_string(),
        }
    }
}

impl ToShortForm for Vec<ChangeHash> {
    fn to_short_form(&self) -> String {
        format!("[{}]", self.iter().map(|h| h.to_short_form()).collect::<Vec<String>>().join(", "))
    }
}

impl ToShortForm for Option<&Vec<ChangeHash>> {
    fn to_short_form(&self) -> String {
        match self {
            Some(change_hashes) => change_hashes.to_short_form(),
            None => "<NONE>".to_string(),
        }
    }
}

pub fn summarize_changes(author: &str, changes: &Vec<ChangedFile>) -> String {
    let added = get_summary_text(&changes, ChangeType::Added, None);
    let removed = get_summary_text(&changes, ChangeType::Removed, None);
    let modified = get_summary_text(&changes, ChangeType::Modified, Some("edited"));

    let strings: Vec<String> = [added, removed, modified]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();

    match strings.len() {
        3 | 0 => format!("{author} made some changes"),
        2 => format!("{author} {} and {}", strings[0], strings[1]),
        1 => format!("{author} {}", strings[0]),
        _ => unreachable!(),
    }
}

fn get_summary_text(
    changes: &Vec<ChangedFile>,
    operation: ChangeType,
    display_operation: Option<&str>,
) -> String {
    let display = display_operation.unwrap_or(match operation {
        ChangeType::Added => "added",
        ChangeType::Removed => "removed",
        ChangeType::Modified => "modified",
    });

    let filtered: Vec<&ChangedFile> = changes
        .iter()
        .filter(|c| c.change_type == operation)
        .collect();

    if filtered.is_empty() {
        return String::new();
    }

    if filtered.len() == 1 {
        // Extract filename via std::path
        let filename = Path::new(&filtered[0].path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(&filtered[0].path);

        return format!("{} {}", display, filename);
    }

    format!("{} {} files", display, filtered.len())
}

pub fn human_readable_timestamp(timestamp: i64) -> String {
    // Current time in ms
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    // Difference in seconds
    let diff = (now - timestamp) / 1000;

	fn pluralize(num: i64, s: &str) -> String {
		if num == 1 {format!("{num} {}", s.to_string())}
		else {format!("{num} {}s", s.to_string())}
	}

    return match diff {
        s if s < 60 => pluralize(s, "second"),
        s if s < 3600 => pluralize(s / 60, "minute"),
        s if s < 86400 => pluralize(s / 3600, "hour"),
        s if s < 604800 => pluralize(s / 86400, "day"),
        s if s < 2_592_000 => pluralize(s / 604_800, "week"),
        s if s < 31_536_000 => pluralize(s / 2_592_000, "month"),
        s => pluralize(s / 31_536_000, "year"),
    } + " ago";
}

pub fn exact_human_readable_timestamp(timestamp: i64) -> String {
    let dt = DateTime::from_timestamp(timestamp / 1000, 0);
    let datetime : DateTime<Local> = DateTime::from(dt.unwrap());
    return datetime.format("%Y-%m-%d %H:%M:%S").to_string();
}

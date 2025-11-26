use std::{
    collections::HashMap, fmt, path::Path, str::FromStr, time::{Duration, SystemTime, UNIX_EPOCH}
};

use crate::{doc_utils::SimpleDocReader, branch::BranchState};
use automerge::{
    Automerge, Change, ChangeHash, Patch, PatchLog, ROOT, ReadDoc, transaction::{CommitOptions, Transaction}
};
use automerge_repo::{DocHandle, DocumentId, PeerConnectionInfo};
use chrono::{DateTime, Local, NaiveDateTime, Utc};
use godot::builtin::Dictionary;
use serde::{Deserialize, Serialize};

// These functions are for compatibilities sake, and they will be removed in the future
#[inline(always)]
pub(crate) fn get_default_patch_log() -> PatchLog {
	#[cfg(not(feature = "automerge_0_6"))]
	{
		PatchLog::inactive()
	}
	#[cfg(feature = "automerge_0_6")]
	{
		PatchLog::inactive(automerge::patches::TextRepresentation::String(automerge::TextEncoding::Utf8CodeUnit))
	}
}

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

pub(crate) fn get_linked_docs_of_branch(
    branch_doc_handle: &DocHandle,
) -> HashMap<String, DocumentId> {
    // Collect all linked doc IDs from this branch
    branch_doc_handle.with_doc(|d| {
        let files = match d.get_obj_id(ROOT, "files") {
            Some(files) => files,
            None => {
                tracing::warn!(
                    "Failed to load files for branch doc {:?}",
                    branch_doc_handle.document_id()
                );
                return HashMap::new();
            }
        };

        d.keys(&files)
            .filter_map(|path| {
                let file = match d.get_obj_id(&files, &path) {
                    Some(file) => file,
                    None => {
                        tracing::error!("Failed to load linked doc {:?}", path);
                        return None;
                    }
                };

                let url = match d.get_string(&file, "url") {
                    Some(url) => url,
                    None => {
                        return None;
                    }
                };

                parse_automerge_url(&url).map(|id| (path.clone(), id))
            })
            .collect::<HashMap<String, DocumentId>>()
    })
}

pub(crate) fn parse_automerge_url(url: &str) -> Option<DocumentId> {
    const PREFIX: &str = "automerge:";
    if !url.starts_with(PREFIX) {
        return None;
    }

    let hash = &url[PREFIX.len()..];
    DocumentId::from_str(hash).ok()
}

pub(crate) fn print_branch_doc(message: &str, doc_handle: &DocHandle) {
    doc_handle.with_doc(|d| {
        let files = d.get_obj_id(ROOT, "files").unwrap();

        let keys = d.keys(files).into_iter().collect::<Vec<_>>();

        tracing::debug!("{:?}: {:?}", message, doc_handle.document_id());

        for key in keys {
            tracing::debug!(" - {:?}", key);
        }
    });
}

pub(crate) fn print_doc(message: &str, doc_handle: &DocHandle) {
    let checked_out_doc_json =
        doc_handle.with_doc(|d| serde_json::to_string(&automerge::AutoSerde::from(d)).unwrap());
    tracing::debug!("{:?}: {:?}", message, checked_out_doc_json);
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MergeMetadata {
    pub merged_branch_id: DocumentId,
    pub merged_at_heads: Vec<ChangeHash>,
    pub forked_at_heads: Vec<ChangeHash>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
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

#[derive(Serialize, Deserialize, Debug)]
pub struct ChangedFile {
    pub change_type: ChangeType,
    pub path: String
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CommitMetadata {
    pub username: Option<String>,
    pub branch_id: Option<DocumentId>,
    pub merge_metadata: Option<MergeMetadata>,
	pub reverted_to: Option<Vec<String>>,
    /// Changed files in this commit. Only valid for commits to branch documents.
    pub changed_files: Option<Vec<ChangedFile>>
}

pub(crate) fn commit_with_attribution_and_timestamp(tx: Transaction, metadata: &CommitMetadata) {
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


pub(crate) fn vec_string_to_heads(heads: Vec<String>) -> Result<Vec<ChangeHash>, String> {
	let mut result = Vec::new();
	for head in heads {
		let change_hash = ChangeHash::from_str(head.as_str());
		if change_hash.is_err() {
			return Err(change_hash.unwrap_err().to_string());
		}
		result.push(change_hash.unwrap());
	}
    Ok(result)
}



pub(crate) fn strategic_waiting(loc: &str) {
	tracing::debug!("pointelssly waiting for about 1 second @ {}", loc);
	let mut count: i32 = 1000;
	while count > 0 {
		std::thread::sleep(Duration::from_millis(100));
		count -= 100;
	}
	tracing::debug!("Done waiting");
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitInfo {
	pub hash: ChangeHash,
	pub prev_change: Option<ChangeHash>,
	pub timestamp: i64,
	pub metadata: Option<CommitMetadata>,
    pub synced: bool,
	pub summary: String
}

impl From<&&Change> for CommitInfo {
	fn from(change: &&Change) -> Self {
		CommitInfo::from(*change)
	}
}

impl From<&Change> for CommitInfo {
	fn from(change: &Change) -> Self {
		CommitInfo {
			hash: change.hash(),
			timestamp: change.timestamp(),
			metadata: change.message().and_then(|m| serde_json::from_str::<CommitMetadata>(&m).ok()),

			// set during ingestion
			synced: false,
			summary: "".to_string(),
			prev_change: None
		}
	}
}

#[derive(Debug)]
pub struct BranchWrapper {
	pub state: BranchState,
	pub children: Vec<DocumentId>
}

#[derive(Debug)]
pub struct DiffWrapper {
	// todo: convert to rust
	pub dict: Dictionary,
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

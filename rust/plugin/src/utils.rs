use std::{
    collections::HashMap,
    str::FromStr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{doc_utils::SimpleDocReader, godot_helpers::{GodotConvertExt, ToGodotExt, ToVariantExt}, godot_project_driver::BranchState};
use automerge::{
    transaction::{CommitOptions, Transaction}, Change, ChangeHash, ReadDoc, ROOT
};
use automerge_repo::{DocHandle, DocumentId, PeerConnectionInfo, RepoHandle};
use godot::{builtin::{dict, Array, Dictionary, GString, PackedStringArray, Variant}, meta::ToGodot, prelude::GodotConvert};
use serde::{Deserialize, Serialize};
use serde_json::Serializer;

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
    pub merged_branch_id: String,
    pub merged_at_heads: Vec<ChangeHash>,
    pub forked_at_heads: Vec<ChangeHash>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CommitMetadata {
    pub username: Option<String>,
    pub branch_id: Option<String>,
    pub merge_metadata: Option<MergeMetadata>,
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
	let last_synced_head = branch_state.synced_heads.last().map(|h| h.to_string()).unwrap_or("<NONE>".to_string());
    tracing::info!(
        "{}: {:?} - linked docs: {:?}, last synced head: {:?}",
        &message, branch_state.name, branch_state.linked_doc_ids.len(), last_synced_head
    );
	tracing::debug!("branch id: {:?}", branch_state.doc_handle.document_id());
	tracing::trace!("linked doc ids: {:?}", branch_state.linked_doc_ids);
	tracing::trace!("synced heads: {:?}", branch_state.synced_heads);
}

pub(crate) fn array_to_heads(packed_string_array: PackedStringArray) -> Vec<ChangeHash> {
    packed_string_array
        .to_vec()
        .iter()
        .map(|h| ChangeHash::from_str(h.to_string().as_str()).unwrap())
        .collect()
}

pub(crate) fn heads_to_array(heads: Vec<ChangeHash>) -> PackedStringArray {
    heads
        .iter()
        .map(|h| GString::from(h.to_string()))
        .collect::<PackedStringArray>()
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
	pub hash: String,
	pub timestamp: i64,
	pub metadata: Option<CommitMetadata>,
}

impl GodotConvert for MergeMetadata {
	type Via = Dictionary;
}

impl ToGodot for MergeMetadata {
	type ToVia<'v> = Dictionary;
	fn to_godot(&self) -> Dictionary {
		dict! {
			"merged_branch_id": self.merged_branch_id.to_godot(),
			"merged_at_heads": self.merged_at_heads.to_godot(),
			"forked_at_heads": self.forked_at_heads.to_godot(),
		}
	}
	fn to_variant(&self) -> Variant {
				dict! {
			"merged_branch_id": self.merged_branch_id.to_godot(),
			"merged_at_heads": self.merged_at_heads.to_godot(),
			"forked_at_heads": self.forked_at_heads.to_godot(),
		}.to_variant()
	}
}

impl GodotConvert for CommitInfo {
	type Via = Dictionary;
}

impl ToGodot for CommitInfo {
	type ToVia<'v> = Dictionary;
	fn to_godot(&self) -> Dictionary {
		let mut md = dict! {
			"hash": self.hash.to_godot(),
			"timestamp": self.timestamp.to_godot(),
		};
		if let Some(metadata) = &self.metadata {
			if let Some(username) = &metadata.username {
				let _ = md.insert("username", username.to_godot());
			}
			if let Some(branch_id) = &metadata.branch_id {
				let _ = md.insert("branch_id", branch_id.to_godot());
			}
			if let Some(merge_metadata) = &metadata.merge_metadata {
				let _ = md.insert("merge_metadata", merge_metadata.to_godot());
			}
		}
		md
	}
	fn to_variant(&self) -> Variant {
		self.to_godot().to_variant()
	}
}

impl From<&&Change> for CommitInfo {
	fn from(change: &&Change) -> Self {
		CommitInfo {
			hash: change.hash().to_string(),
			timestamp: change.timestamp(),
			metadata: change.message().and_then(|m| serde_json::from_str::<CommitMetadata>(&m).ok()),
		}
	}
}

fn branch_state_to_dict(branch_state: &BranchState) -> Dictionary {
    let mut branch = dict! {
        "name": branch_state.name.clone(),
        "id": branch_state.doc_handle.document_id().to_string(),
        "is_main": branch_state.is_main,

        // we shouldn't have branches that don't have any changes but sometimes
        // the branch docs are not synced correctly so this flag is used in the UI to
        // indicate that the branch is not loaded and prevent users from checking it out
        "is_not_loaded": branch_state.doc_handle.with_doc(|d| d.get_heads().len() == 0),
        "heads": heads_to_array(branch_state.synced_heads.clone()),
        "is_merge_preview": branch_state.merge_info.is_some(),
    };

    if let Some(fork_info) = &branch_state.fork_info {
        let _ = branch.insert("forked_from", fork_info.forked_from.to_string());
        let _ = branch.insert("forked_at", heads_to_array(fork_info.forked_at.clone()));
    }

    if let Some(merge_info) = &branch_state.merge_info {
        let _ = branch.insert("merge_into", merge_info.merge_into.to_string());
        let _ = branch.insert("merge_at", heads_to_array(merge_info.merge_at.clone()));
    }

    branch
}

impl GodotConvert for BranchState {
	type Via = Dictionary;
}

impl ToGodot for BranchState {
	type ToVia<'v> = Dictionary;
	fn to_godot(&self) -> Dictionary {
		branch_state_to_dict(self)
	}
}

impl ToVariantExt for Option<BranchState> {
	fn _to_variant(&self) -> Variant {
		match self {
			Some(branch_state) => branch_state.to_godot().to_variant(),
			None => Variant::nil(),
		}
	}
}

impl ToVariantExt for Option<&BranchState> {
	fn _to_variant(&self) -> Variant {
		match self {
			Some(branch_state) => branch_state.to_godot().to_variant(),
			None => Variant::nil(),
		}
	}
}


fn peer_connection_info_to_dict(peer_connection_info: &PeerConnectionInfo) -> Dictionary {
    let mut doc_sync_states = Dictionary::new();

    for (doc_id, doc_state) in peer_connection_info.docs.iter() {
        let last_received = doc_state
            .last_received
            .map(system_time_to_variant)
            .unwrap_or(Variant::nil());

        let last_sent = doc_state
            .last_sent
            .map(system_time_to_variant)
            .unwrap_or(Variant::nil());

        let last_sent_heads = doc_state
            .last_sent_heads
            .as_ref()
            .map(|heads| heads_to_array(heads.clone()).to_variant())
            .unwrap_or(Variant::nil());

        let last_acked_heads = doc_state
            .last_acked_heads
            .as_ref()
            .map(|heads| heads_to_array(heads.clone()).to_variant())
            .unwrap_or(Variant::nil());

        let _ = doc_sync_states.insert(
            doc_id.to_string(),
            dict! {
                "last_received": last_received,
                "last_sent": last_sent,
                "last_sent_heads": last_sent_heads,
                "last_acked_heads": last_acked_heads,
            },
        );
    }

    let last_received = peer_connection_info
        .last_received
        .map(system_time_to_variant)
        .unwrap_or(Variant::nil());

    let last_sent = peer_connection_info
        .last_sent
        .map(system_time_to_variant)
        .unwrap_or(Variant::nil());

    let is_connected = !last_received.is_nil();

    dict! {
        "doc_sync_states": doc_sync_states,
        "last_received": last_received,
        "last_sent": last_sent,
        "is_connected": is_connected,
    }
}

fn system_time_to_variant(time: SystemTime) -> Variant {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs().to_variant())
        .unwrap_or(Variant::nil())
}

impl GodotConvertExt for PeerConnectionInfo {
	type Via = Dictionary;
}

impl ToGodotExt for PeerConnectionInfo {
	type ToVia<'v> = Dictionary;
	fn _to_godot(&self) -> Self::ToVia<'_> {
		peer_connection_info_to_dict(self)
	}
	fn _to_variant(&self) -> Variant {
		peer_connection_info_to_dict(self).to_variant()
	}
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
        self.iter().map(|h| h.to_short_form()).collect::<Vec<String>>().join(", ")
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
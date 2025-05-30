use std::{
    collections::HashMap,
    str::FromStr,
    time::{Duration, Instant, SystemTime},
};

use crate::{doc_utils::SimpleDocReader, godot_project_driver::BranchState};
use automerge::{
    transaction::{CommitOptions, Transaction},
    ChangeHash, ReadDoc, ROOT,
};
use automerge_repo::{DocHandle, DocumentId, RepoHandle};
use godot::builtin::{Array, GString, PackedStringArray};
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
                println!(
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
                        println!("Failed to load linked doc {:?}", path);
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

        println!("{:?}: {:?}", message, doc_handle.document_id());

        for key in keys {
            println!("  {:?}", key);
        }
    });
}

pub(crate) fn print_doc(message: &str, doc_handle: &DocHandle) {
    let checked_out_doc_json =
        doc_handle.with_doc(|d| serde_json::to_string(&automerge::AutoSerde::from(d)).unwrap());
    println!("rust: {:?}: {:?}", message, checked_out_doc_json);
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
    println!(
        "rust: {:?}: {:?} {:?} {:?}",
        message, branch_state.name, branch_state.linked_doc_ids, branch_state.synced_heads
    );
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
	println!("pointelssly waiting for about 1 second @ {}", loc);
	let mut count: i32 = 1000;
	while count > 0 {
		std::thread::sleep(Duration::from_millis(100));
		count -= 100;
	}
	println!("Done waiting");
}
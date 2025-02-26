use std::{
    collections::HashMap,
    str::FromStr,
    time::{Instant, SystemTime},
};

use crate::{doc_utils::SimpleDocReader, godot_project_driver::BranchState};
use automerge::{
    transaction::{CommitOptions, Transaction},
    ChangeHash, ReadDoc, ROOT,
};
use automerge_repo::{DocHandle, DocumentId, RepoHandle};
use godot::builtin::PackedStringArray;

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

pub(crate) fn commit_with_attribution_and_timestamp(tx: Transaction, name: &Option<String>) {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    match name {
        Some(name) => {
            tx.commit_with(
                CommitOptions::default()
                    .with_message(name)
                    .with_time(timestamp),
            );

            println!("commit with name and timestamp: {:?}", name);
        }
        None => {
            tx.commit_with(CommitOptions::default().with_time(timestamp));
        }
    }
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

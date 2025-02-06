use std::str::FromStr;

use automerge::{ReadDoc, ROOT};
use automerge_repo::{DocHandle, DocumentId};
use crate::doc_utils::SimpleDocReader;

pub(crate) fn get_linked_docs_of_branch(branch_doc_handle: &DocHandle) -> Vec<DocumentId> {
  // Collect all linked doc IDs from this branch
  branch_doc_handle.with_doc(|d| {
      let files = match d.get_obj_id(ROOT, "files") {
          Some(files) => files,
          None => {
              println!("Failed to load files for branch doc {:?}", branch_doc_handle.document_id());
              return vec![];
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

              parse_automerge_url(&url)
          })
          .collect::<Vec<_>>()
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
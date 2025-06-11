use std::collections::HashSet;

pub fn get_changed_files_vec(patches: &Vec<automerge::Patch>) -> Vec<String> {
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

    return changed_files.iter().cloned().collect::<Vec<String>>();
}

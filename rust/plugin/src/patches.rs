use std::collections::HashSet;

use godot::{
    builtin::{dict, Dictionary, GString, PackedStringArray},
    meta::ToGodot,
};

use crate::godot_scene::{self, PackedGodotScene};

pub fn get_changed_files_vec(patches: Vec<automerge::Patch>) -> Vec<String> {
    let mut changed_files = HashSet::new();

    // log all patches
    for patch in patches.clone() {
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

        println!("changed files: {:?}", changed_files);
    }

    return changed_files
        .iter()
        .cloned()
        .collect::<Vec<String>>();
}

pub fn get_changed_files(patches: Vec<automerge::Patch>) -> PackedStringArray {
    let mut changed_files = HashSet::new();

    // log all patches
    for patch in patches.clone() {
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

        println!("changed files: {:?}", changed_files);
    }

    return changed_files
        .iter()
        .map(|s| GString::from(s))
        .collect::<PackedStringArray>();
}

enum FileUpdate {
    Patch {
        path: String,
        patch: automerge::Patch,
        scene: PackedGodotScene,
    },
    Reload {
        path: String,
        content: String,
    },
}

fn get_changes_patch(updates: Vec<FileUpdate>) -> Vec<Dictionary> {
    let mut patches = vec![];
    for file_update in updates {
        match file_update {
            FileUpdate::Patch { path, patch, scene } => {
                match patch.action {
                    // handle update node
                    automerge::PatchAction::PutMap {
                        key,
                        value,
                        conflict: _,
                    } => match (patch.path.get(0), patch.path.get(1), patch.path.get(2)) {
                        (
                            Some((_, automerge::Prop::Map(maybe_nodes))),
                            Some((_, automerge::Prop::Map(node_path))),
                            Some((_, automerge::Prop::Map(prop_or_attr))),
                        ) => {
                            if maybe_nodes == "nodes" {
                                if let automerge::Value::Scalar(v) = value.0 {
                                    if let automerge::ScalarValue::Str(smol_str) = v.as_ref() {
                                        let string_value = smol_str.to_string();

                                        let mut dict = dict! {
                                            "file_path": path,
                                            "node_path": node_path.to_variant(),
                                            "type": if prop_or_attr == "properties" {
                                                "property_changed"
                                            } else {
                                                "attribute_changed"
                                            },
                                            "key": key,
                                            "value": string_value,
                                        };

                                        // Look up node in scene and get instance / type attribute if it exists
                                        if let Some(node) =
                                            godot_scene::get_node_by_path(&scene, node_path)
                                        {
                                            let attributes =
                                                godot_scene::get_node_attributes(&node);
                                            if let Some(instance) = attributes.get("instance") {
                                                let _ =
                                                    dict.insert("instance_path", instance.clone());
                                            } else if let Some(type_val) = attributes.get("type") {
                                                let _ =
                                                    dict.insert("instance_type", type_val.clone());
                                            }
                                        }
                                        patches.push(dict);
                                    }
                                }
                            }
                        }
                        _ => {}
                    },

                    // handle delete node
                    automerge::PatchAction::DeleteMap { key: node_path } => {
                        if patch.path.len() != 1 {
                            continue;
                        }
                        match patch.path.get(0) {
                            Some((_, automerge::Prop::Map(key))) => {
                                if key == "nodes" {
                                    patches.push(dict! {
                                        "file_path": path,
                                        "node_path": node_path.to_variant(),
                                        "type": "node_deleted",
                                    });
                                }
                            }
                            _ => {}
                        };
                    }
                    _ => {}
                }
            }
            FileUpdate::Reload { path, content } => {
                patches.push(dict! {
                    "file_path": path,
                    "type": "file_reloaded",
                    "content": content,
                });
            }
        }
    }
    patches
}

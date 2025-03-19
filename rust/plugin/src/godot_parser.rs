use automerge::{
    transaction::{Transactable, Transaction},
    ObjType, ROOT,
};
use autosurgeon::{Hydrate, Reconcile};
use std::collections::HashMap;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::doc_utils::SimpleDocReader;

#[derive(Debug, Clone)]
enum NodeTreeType {
    Scene,
    Resource,
}

#[derive(Debug, Clone)]
pub struct GodotNodeTree {
    tree_type: NodeTreeType,
    format: u64,
    uid: String,
    nodes: HashMap<String, GodotNode>,
    ext_resources: HashMap<String, GodotNode>,
    sub_resources: HashMap<String, GodotNode>,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct GodotNode {
    attributes: HashMap<String, String>, // key value pairs in the header of the section
    properties: HashMap<String, String>, // key value pairs below the section header
}

impl GodotNodeTree {
    pub fn reconcile(&self, tx: &mut Transaction, path: String) {
        let files = tx
            .get_obj_id(ROOT, "files")
            .unwrap_or_else(|| panic!("Could not find files object in document"));

        let scene_file = tx
            .get_obj_id(&files, &path)
            .unwrap_or_else(|| tx.put_object(&files, &path, ObjType::Map).unwrap());

        let structured_content = tx
            .get_obj_id(&scene_file, "structured_content")
            .unwrap_or_else(|| {
                tx.put_object(&scene_file, "structured_content", ObjType::Map)
                    .unwrap()
            });

        let nodes = tx
            .get_obj_id(&structured_content, "nodes")
            .unwrap_or_else(|| {
                tx.put_object(&structured_content, "nodes", ObjType::Map)
                    .unwrap()
            });

        for (path, node) in &self.nodes {
            let node_key = tx
                .get_obj_id(&nodes, path)
                .unwrap_or_else(|| tx.put_object(&nodes, path, ObjType::Map).unwrap());

            for (key, value) in &node.attributes {
                tx.put(&node_key, key, value);
            }

            for (key, value) in &node.properties {
                tx.put(&node_key, key, value);
            }
        }
    }
}

// WIP custom reconciler
/*
fn get_string(value: automerge::Value) -> Option<String> {
    match value {
        automerge::Value::Scalar(v) => match v.as_ref() {
            automerge::ScalarValue::Str(smol_str) => Some(smol_str.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn assign<R: autosurgeon::Reconciler>(
    m: &mut <R as Reconciler>::Map<'_>,
    key: &str,
    value: String,
) {
    let value_clone = value.clone();
    match m.entry(key) {
        Some(v) => {
            if get_string(v) != Some(value) {
                m.put(key, value_clone);
            }
        }
        None => {
            m.put(key, value);
        }
    };
}

impl Reconcile for GodotSceneNode {
    type Key<'a> = u64;

    fn reconcile<R: autosurgeon::Reconciler>(&self, reconciler: R) -> Result<(), R::Error> {
        let mut m: <R as Reconciler>::Map<'_> = reconciler.map()?;

        assign(&mut m, "name", self.name.clone());
        assign(&mut m, "parent", self.parent.clone());
        assign(&mut m, "instance", self.instance.clone());

        let name_entry = m.entry("name");

        Ok(())
    }
}*/

pub fn parse(source: &String) -> Result<GodotNodeTree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(tree_sitter_godot_resource::language())
        .expect("Error loading godot resource grammar");

    let result = parser.parse(source, None);

    /*println!(
        "Tree s-expression:\n{}",
        result.clone().unwrap().root_node().to_sexp()
    );*/

    return match result {
        Some(tree) => {
            let content_bytes = source.as_bytes();
            // Query for section attributes and paths
            let query = "(section
                (identifier) @section_id
                (attribute 
                    (identifier) @attr_key 
                    (_) @attr_value)*
                (property 
                    (path) @prop_key 
                    (_) @prop_value)*
            )";
            let query =
                Query::new(tree_sitter_godot_resource::language(), query).expect("Invalid query");
            let mut query_cursor = QueryCursor::new();
            let matches = query_cursor.matches(&query, tree.root_node(), content_bytes);
            let mut scene = GodotNodeTree {
                tree_type: NodeTreeType::Scene,
                format: 3,
                uid: String::new(),
                nodes: HashMap::new(),
                ext_resources: HashMap::new(),
                sub_resources: HashMap::new(),
            };

            for m in matches {
                let mut attributes = HashMap::new();
                let mut properties = HashMap::new();
                let mut section_id = String::new();

                for (i, capture) in m.captures.iter().enumerate() {
                    if let Ok(text) = capture.node.utf8_text(content_bytes) {
                        match capture.index {
                            0 => {
                                // section_id
                                section_id = text.to_string();
                            }
                            1 => {
                                // attr_key
                                if let Some(value_capture) = m.captures.get(i + 1) {
                                    if let Ok(value) = value_capture.node.utf8_text(content_bytes) {
                                        attributes.insert(text.to_string(), value.to_string());
                                    }
                                }
                            }
                            3 => {
                                // prop_key
                                if let Some(value_capture) = m.captures.get(i + 1) {
                                    if let Ok(value) = value_capture.node.utf8_text(content_bytes) {
                                        properties.insert(text.to_string(), value.to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                println!("section_id: {}", section_id);
                println!("attributes: {:?}", attributes);
                println!("properties: {:?}", properties);

                // let attributes_clone = attributes.clone();
                // let node = GodotSceneNode {
                //     attributes,
                //     properties,
                // };

                // if section_id == "node" {
                //     let mut node_clone = node.clone();
                //     let scene_clone = scene.clone();

                //     // parse instance property to path instead of local id
                //     if let Some(instance) = node_clone.attributes.get("instance") {
                //         println!("has instance {}", instance);
                //         if let Some(path) = external_resource_to_path(instance, &scene) {
                //             println!("parse {}", path);
                //             node_clone.attributes.insert("instance".to_string(), path);
                //         } else {
                //             println!("can't parse");
                //         }
                //     }

                //     if let Some(node_path) = get_node_path(scene_clone, node) {
                //         scene.nodes.insert(node_path, node_clone);
                //     }
                // } else if section_id == "editable_path" {
                //     let editable_path = node.properties.get("path").unwrap();
                //     scene.editable_paths.push(editable_path.clone());
                // } else if section_id == "ext_resource" {
                //     let node_clone = node.clone();
                //     if let Some(raw_id) = attributes_clone.get("id") {
                //         let id = raw_id.to_string()[1..raw_id.len() - 1].to_string();

                //         scene.external_resources.insert(id, node_clone);
                //     }
                // } else if section_id == "connection" {
                //     let connections = GodotSceneConnections {
                //         attributes: attributes_clone.clone(),
                //     };

                //     let mut connection_id = String::new();
                //     if let Some(signal) = connections.attributes.get("signal") {
                //         connection_id.push_str(signal);
                //     }
                //     if let Some(target) = connections.attributes.get("target") {
                //         connection_id.push_str(target);
                //     }
                //     if let Some(method) = connections.attributes.get("method") {
                //         connection_id.push_str(method);
                //     }

                //     scene.connections.insert(connection_id, connections);
                // } else if section_id == "subresource" {
                //     let node_clone = node.clone();
                //     if let Some(raw_id) = attributes_clone.get("id") {
                //         let id = raw_id.to_string()[1..raw_id.len() - 1].to_string();
                //         scene.internal_resources.insert(id, node_clone);
                //     } else {
                //         // something? internal resources always have an id
                //     }
                // }
            }

            //  println!("scene {:#?}", scene);

            Ok(scene)
        }
        None => Err("Failed to parse scene file".to_string()),
    };
}

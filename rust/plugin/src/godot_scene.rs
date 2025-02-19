use autosurgeon::{Hydrate, Reconcile};
use std::collections::HashMap;
use tree_sitter::{Parser, Query, QueryCursor};

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct PackedGodotScene {
    format: u64,
    uid: String,
    script_class: String,
    attributes: HashMap<String, String>,
    nodes: HashMap<String, GodotSceneNode>,
    external_resources: HashMap<String, GodotSceneNode>,
    internal_resources: HashMap<String, GodotSceneNode>,
    connections: HashMap<String, GodotSceneConnections>,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct PackedGodotResource {
    format: u64,
    uid: String,
    script_class: String,
    godot_type: String,
    attributes: HashMap<String, String>,
    external_resources: HashMap<String, GodotSceneNode>,
    internal_resources: HashMap<String, GodotSceneNode>,
}


#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct GodotSceneConnections {
    attributes: HashMap<String, String>, // key value pairs in the header of the section
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct GodotSceneNode {
    
    attributes: HashMap<String, String>, // key value pairs in the header of the section
    properties: HashMap<String, String>, // key value pairs below the section header
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

pub fn parse(source: &String) -> Result<PackedGodotScene, String> {
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
            let mut scene = PackedGodotScene {
                attributes: HashMap::new(),
                nodes: HashMap::new(),
                external_resources: HashMap::new(),
                internal_resources: HashMap::new(),
                connections: HashMap::new(),
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
                                        if let Some(path) = external_resource_to_path(value, &scene)
                                        {
                                            properties.insert(text.to_string(), path);
                                        } else {
                                            properties.insert(text.to_string(), value.to_string());
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                let attributes_clone = attributes.clone();
                let node = GodotSceneNode {
                    attributes,
                    properties,
                };

                if section_id == "node" {
                    let mut node_clone = node.clone();
                    let scene_clone = scene.clone();

                    // parse instance property to path instead of local id
                    if let Some(instance) = node_clone.attributes.get("instance") {
                        println!("has instance {}", instance);
                        if let Some(path) = external_resource_to_path(instance, &scene) {
                            println!("parse {}", path);
                            node_clone.attributes.insert("instance".to_string(), path);
                        } else {
                            println!("can't parse");
                        }
                    }

                    if let Some(node_path) = get_node_path(scene_clone, node) {
                        scene.nodes.insert(node_path, node_clone);
                    }
                } else if section_id == "ext_resource" {
                    let node_clone = node.clone();
                    if let Some(raw_id) = attributes_clone.get("id") {
                        let id = raw_id.to_string()[1..raw_id.len() - 1].to_string();

                        scene.external_resources.insert(id, node_clone);
                    }
                } else if section_id == "connection" {
                    let connections = GodotSceneConnections {
                        attributes: attributes_clone.clone(),
                    };

                    let mut connection_id = String::new();
                    if let Some(signal) = connections.attributes.get("signal") {
                        connection_id.push_str(signal);
                    }
                    if let Some(target) = connections.attributes.get("target") {
                        connection_id.push_str(target);
                    }
                    if let Some(method) = connections.attributes.get("method") {
                        connection_id.push_str(method);
                    }

                    scene.connections.insert(connection_id, connections);
                } else if section_id == "subresource" {
                    let node_clone = node.clone();
                    if let Some(raw_id) = attributes_clone.get("id") {
                        let id = raw_id.to_string()[1..raw_id.len() - 1].to_string();
                        scene.internal_resources.insert(id, node_clone);
                    } else {
                        // something? internal resources always have an id
                    }
                }
            }

            //  println!("scene {:#?}", scene);

            Ok(scene)
        }
        None => Err("Failed to parse scene file".to_string()),
    };
}

fn external_resource_to_path(value: &str, scene: &PackedGodotScene) -> Option<String> {
    if value.starts_with("ExtResource(\"") && value.ends_with("\")") {
        let id = &value[13..value.len() - 2];
        if let Some(ext_resource) = scene.external_resources.get(id) {
            if let Some(path) = ext_resource.attributes.get("path") {
                return Some(path[1..path.len() - 1].to_string());
            }
        }
    }
    None
}

fn get_node_path(scene: PackedGodotScene, node: GodotSceneNode) -> Option<String> {
    // Get the current node's name

    let scene_clone = scene.clone();
    let node_clone = node.clone();

    if let Some(name) = get_node_name(node_clone) {
        // Base case - if parent is "." or no parent, just return name
        match get_node_parent(node) {
            None => None,
            Some(parent_name) => {
                // Look up parent node in scene
                if let Some(parent_node) = scene.nodes.get(&parent_name) {
                    // Recursively get parent's path and combine
                    if let Some(parent_path) = get_node_path(scene_clone, parent_node.clone()) {
                        Some(format!("{}/{}", parent_path, name))
                    } else {
                        Some(name)
                    }
                } else {
                    Some(name)
                }
            }
        }
    } else {
        None
    }
}

fn get_node_parent(node: GodotSceneNode) -> Option<String> {
    node.attributes
        .get("parent")
        .map(|p| p[1..p.len() - 1].to_string())
}

fn get_node_name(node: GodotSceneNode) -> Option<String> {
    node.attributes
        .get("name")
        .map(|n| n[1..n.len() - 1].to_string())
}

pub fn get_node_by_path(scene: &PackedGodotScene, path: &str) -> Option<GodotSceneNode> {
    scene.nodes.get(path).cloned()
}

pub fn get_node_attributes(node: &GodotSceneNode) -> HashMap<String, String> {
    node.attributes.clone()
}

pub fn get_node_properties(node: &GodotSceneNode) -> HashMap<String, String> {
    node.properties.clone()
}

pub fn get_external_resource_by_id(scene: &PackedGodotScene, id: &str) -> Option<GodotSceneNode> {
    scene.external_resources.get(id).cloned()
}

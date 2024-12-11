use autosurgeon::{Hydrate, Reconcile};
use std::collections::HashMap;
use tree_sitter::{Parser, Query, QueryCursor};

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct PackedGodotScene {
    gd_scene: GodotSceneNode,
    nodes: HashMap<String, GodotSceneNode>,
    external_resources: HashMap<String, GodotSceneNode>,
    // todo: parse sub resources and connections
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
                gd_scene: GodotSceneNode {
                    attributes: HashMap::new(),
                    properties: HashMap::new(),
                },
                nodes: HashMap::new(),
                external_resources: HashMap::new(),
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
                        if let Some(path) = external_resource_to_path(instance, &scene) {
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
                } else if section_id == "gd_scene" {
                    scene.gd_scene = node
                }
            }

            //  println!("scene {:#?}", scene);

            Ok(scene)
        }
        None => Err("Failed to parse scene file".to_string()),
    };
}

pub fn serialize(scene: PackedGodotScene) -> String {
    let mut output = String::new();

    // Write gd_scene header
    output.push_str("[gd_scene");
    for (key, value) in scene.gd_scene.attributes {
        output.push_str(&format!(" {}={}", key, value));
    }
    output.push_str("]\n\n");

    // Write external resources
    for (id, resource) in scene.external_resources.clone() {
        output.push_str("[ext_resource");
        for (key, value) in resource.attributes {
            output.push_str(&format!(" {}={}", key, value));
        }
        output.push_str("]\n");
    }
    output.push_str("\n");

    // Write nodes in parent-child order
    let mut written_nodes = std::collections::HashSet::new();
    let mut pending_nodes = scene.nodes.clone();

    // First find and write the root node (node without parent attribute)
    let mut root_path = None;
    for (path, node) in pending_nodes.iter() {
        if !node.attributes.contains_key("parent") {
            root_path = Some(path.clone());
            break;
        }
    }

    if let Some(root_path) = root_path {
        let root_node = pending_nodes.remove(&root_path).unwrap();

        output.push_str("[node");
        for (key, value) in &root_node.attributes {
            output.push_str(&format!(" {}={}", key, value));
        }
        output.push_str("]\n");

        for (key, value) in root_node.properties {
            let translated_value = path_to_external_resource(&value, &scene.external_resources);
            output.push_str(&format!("{}={}\n", key, translated_value));
        }
        output.push_str("\n");

        written_nodes.insert(root_path);
    }

    while !pending_nodes.is_empty() {
        let mut nodes_to_write = Vec::new();

        // Find nodes that can be written (parent exists)
        for (path, node) in pending_nodes.iter() {
            let parent_exists = match node.attributes.get("parent") {
                Some(parent) if parent == "\".\"" => {
                    // Only write direct children of root if root is written
                    written_nodes.iter().any(|p| !p.contains('/'))
                }
                Some(_) => {
                    // Check if parent path exists
                    let parent_path = path.rsplit_once('/').map(|(p, _)| p.to_string());
                    match parent_path {
                        Some(p) => written_nodes.contains(&p),
                        None => false,
                    }
                }
                None => false, // Root node was already written
            };

            if parent_exists {
                nodes_to_write.push(path.clone());
            }
        }

        // Write the nodes we found
        for path in nodes_to_write {
            let node = pending_nodes.remove(&path).unwrap();

            output.push_str("[node");

            // Handle instance path translation
            let mut attributes = node.attributes.clone();
            if let Some(instance) = attributes.get("instance") {
                let translated = path_to_external_resource(instance, &scene.external_resources);
                attributes.insert("instance".to_string(), translated);
            }

            for (key, value) in attributes {
                output.push_str(&format!(" {}={}", key, value));
            }
            output.push_str("]\n");

            // Write node properties
            for (key, value) in node.properties {
                let translated_value = path_to_external_resource(&value, &scene.external_resources);
                output.push_str(&format!("{}={}\n", key, translated_value));
            }
            output.push_str("\n");

            written_nodes.insert(path);
        }
    }

    output
}

fn path_to_external_resource(
    instance: &str,
    external_resources: &HashMap<String, GodotSceneNode>,
) -> String {
    if instance.starts_with("res://") {
        // Find matching external resource
        for (id, resource) in external_resources {
            if let Some(res_path) = resource.attributes.get("path") {
                let quoted_instance = format!("\"{}\"", instance);
                if quoted_instance == *res_path {
                    return format!("ExtResource(\"{}\")", id);
                }
            }
        }
    }
    instance.to_string()
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
            None => Some(name),
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

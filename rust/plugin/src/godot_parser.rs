use automerge::{
    transaction::{Transactable, Transaction},
    ObjType, ROOT,
};
use autosurgeon::{Hydrate, Reconcile};
use std::collections::HashMap;
use tree_sitter::{Parser, Query, QueryCursor};
use uuid;

use crate::doc_utils::SimpleDocReader;

#[derive(Debug, Clone)]
enum NodeTreeType {
    Scene,
    Resource,
}

#[derive(Debug, Clone)]
pub struct GodotScene {
    attributes: HashMap<String, String>,
    nodes: HashMap<String, GodotNode>,
    ext_resources: HashMap<String, GodotNode>,
    sub_resources: HashMap<String, GodotNode>,
    root_node_id: String,
}

pub struct GodotResource {
    attributes: HashMap<String, String>,
    nodes: HashMap<String, GodotNode>,
    ext_resources: HashMap<String, GodotNode>,
    sub_resources: HashMap<String, GodotNode>,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
pub struct GodotNode {
    id: String,
    attributes: HashMap<String, String>, // key value pairs in the header of the section
    properties: HashMap<String, String>, // key value pairs below the section header
    child_node_ids: Vec<String>,
}

impl GodotScene {
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

    pub fn serialize(&self) -> String {
        let mut output = String::new();

        // Scene header
        if let Some(format) = self.attributes.get("format") {
            if let Some(uid) = self.attributes.get("uid") {
                output.push_str(&format!(
                    "[gd_scene load_steps={} format={} {}]\n\n",
                    self.ext_resources.len() + self.sub_resources.len() + 1,
                    format,
                    uid
                ));
            } else {
                output.push_str(&format!(
                    "[gd_scene load_steps={} format={}]\n\n",
                    self.ext_resources.len() + self.sub_resources.len() + 1,
                    format
                ));
            }
        } else {
            // Default header if no format specified
            output.push_str("[gd_scene]\n\n");
        }

        // External resources
        for (_, resource) in &self.ext_resources {
            output.push_str("[ext_resource ");

            // Attributes
            let mut attrs = Vec::new();
            for (key, value) in &resource.attributes {
                if key != "id" {
                    // id is handled separately
                    attrs.push(format!("{}={}", key, value));
                }
            }

            // Ensure id is the last attribute
            if let Some(id) = resource.attributes.get("id") {
                attrs.push(format!("id={}", id));
            }

            output.push_str(&attrs.join(" "));
            output.push_str("]\n");
        }

        if !self.ext_resources.is_empty() {
            output.push('\n');
        }

        // Sub-resources
        for (_, resource) in &self.sub_resources {
            output.push_str("[sub_resource ");

            // Attributes
            let mut attrs = Vec::new();
            for (key, value) in &resource.attributes {
                if key != "id" {
                    // id is handled separately
                    attrs.push(format!("{}={}", key, value));
                }
            }

            // Ensure id is the last attribute
            if let Some(id) = resource.attributes.get("id") {
                attrs.push(format!("id={}", id));
            }

            output.push_str(&attrs.join(" "));
            output.push_str("]\n");

            // Properties
            for (key, value) in &resource.properties {
                if !key.starts_with("metadata/patchwork_id") {
                    // Skip patchwork IDs
                    output.push_str(&format!("{}={}\n", key, value));
                }
            }

            output.push('\n');
        }

        // Nodes - we need to traverse in the correct order
        if !self.nodes.is_empty() {
            if let Some(root_node) = self.nodes.get(&self.root_node_id) {
                self.serialize_node(&mut output, root_node, "", &self.nodes);
            }
        }

        output
    }

    fn serialize_node(
        &self,
        output: &mut String,
        node: &GodotNode,
        parent_path: &str,
        nodes: &HashMap<String, GodotNode>,
    ) {
        output.push_str("[node ");

        // Attributes
        let mut attrs = Vec::new();

        // Ensure name is the first attribute
        if let Some(name) = node.attributes.get("name") {
            attrs.push(format!("name={}", name));
        }

        // Add type if present
        if let Some(node_type) = node.attributes.get("type") {
            attrs.push(format!("type={}", node_type));
        }

        // Add parent if not root
        if !parent_path.is_empty() {
            attrs.push(format!("parent=\"{}\"", parent_path));
        }

        // Add remaining attributes (except name, type, parent which were handled above)
        for (key, value) in &node.attributes {
            if key != "name" && key != "type" && key != "parent" {
                attrs.push(format!("{}={}", key, value));
            }
        }

        output.push_str(&attrs.join(" "));
        output.push_str("]\n");

        // Properties
        for (key, value) in &node.properties {
            output.push_str(&format!("{}={}\n", key, value));
        }

        // Always add a blank line after a node's properties
        output.push('\n');

        // Process children
        let node_name = node
            .attributes
            .get("name")
            .map(|n| n.trim_matches('"'))
            .unwrap_or("");

        let new_parent_path = if parent_path.is_empty() {
            node_name.to_string()
        } else {
            format!("{}/{}", parent_path, node_name)
        };

        // Recursively serialize children
        for child_id in &node.child_node_ids {
            if let Some(child_node) = nodes.get(child_id) {
                self.serialize_node(output, child_node, &new_parent_path, nodes);
            }
        }
    }
}

pub fn parse_scene(source: &String) -> Result<GodotScene, String> {
    let mut parser = Parser::new();
    parser
        .set_language(tree_sitter_godot_resource::language())
        .expect("Error loading godot resource grammar");

    let result = parser.parse(source, None);

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

            let mut scene_attributes: HashMap<String, String> = HashMap::new();
            let mut nodes: HashMap<String, GodotNode> = HashMap::new();
            let mut ext_resources: HashMap<String, GodotNode> = HashMap::new();
            let mut sub_resources: HashMap<String, GodotNode> = HashMap::new();
            let mut root_node_id = String::new();

            // Stack to track node hierarchy
            let mut node_stack: Vec<String> = Vec::new();

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

                // Process the section based on its type
                if section_id.is_empty() {
                    // First section with no ID is the scene attributes
                    scene_attributes = attributes;
                } else if section_id == "node" {
                    // Create a node and add it to the nodes map
                    let mut node_id = String::new();

                    // Check if node has a patchwork_id in metadata
                    if let Some(patchwork_id) = properties.get("metadata/patchwork_id") {
                        node_id = patchwork_id.clone();
                    } else {
                        // Generate a UUID if no patchwork_id exists
                        node_id = uuid::Uuid::new_v4().simple().to_string();
                        properties.insert("metadata/patchwork_id".to_string(), node_id.clone());
                    }

                    // If this is the first node, it's the root node
                    if root_node_id.is_empty() {
                        root_node_id = node_id.clone();
                        node_stack.push(node_id.clone());
                    } else {
                        // Handle parent-child relationships
                        if let Some(parent_path) = attributes.get("parent") {
                            // Handle different parent path formats
                            if parent_path == "\".\"" {
                                // Parent is root node
                                if !node_stack.is_empty() {
                                    // Pop until we reach the root node
                                    while node_stack.len() > 1 {
                                        node_stack.pop();
                                    }
                                }
                            } else if parent_path.starts_with("\"") && parent_path.ends_with("\"") {
                                // Extract parent name from the path
                                let parent_name = parent_path.trim_matches('"');

                                // Handle relative paths
                                if parent_name.contains("/") {
                                    // For simplicity, we'll just pop to root for complex paths
                                    // A more complete implementation would navigate the path
                                    while node_stack.len() > 1 {
                                        node_stack.pop();
                                    }
                                } else {
                                    // Find the parent node in the stack
                                    let mut found = false;
                                    while !node_stack.is_empty() {
                                        let potential_parent_id = node_stack.last().unwrap();
                                        if let Some(potential_parent) =
                                            nodes.get(potential_parent_id)
                                        {
                                            // Compare the node name with the parent path
                                            if let Some(node_name) =
                                                potential_parent.attributes.get("name")
                                            {
                                                if node_name == parent_path {
                                                    found = true;
                                                    break;
                                                }
                                            }
                                        }
                                        node_stack.pop();
                                    }

                                    // If parent not found, default to root
                                    if !found && !node_stack.is_empty() {
                                        while node_stack.len() > 1 {
                                            node_stack.pop();
                                        }
                                    }
                                }
                            }

                            // Add this node as a child of the current parent
                            if let Some(parent_id) = node_stack.last() {
                                if let Some(parent_node) = nodes.get_mut(parent_id) {
                                    parent_node.child_node_ids.push(node_id.clone());
                                }
                            }
                        }

                        // Push this node onto the stack
                        node_stack.push(node_id.clone());
                    }

                    let node = GodotNode {
                        id: node_id.clone(),
                        attributes,
                        properties,
                        child_node_ids: Vec::new(), // Child relationships are processed above
                    };

                    nodes.insert(node_id, node);
                } else if section_id == "ext_resource" {
                    // Add to ext_resources map
                    if let Some(id) = attributes.get("id").cloned() {
                        let node = GodotNode {
                            id: id.clone(),
                            attributes,
                            properties,
                            child_node_ids: Vec::new(),
                        };
                        ext_resources.insert(id.clone(), node);
                    }
                } else if section_id == "sub_resource" {
                    // Add to sub_resources map
                    if let Some(id) = attributes.get("id").cloned() {
                        let node = GodotNode {
                            id: id.clone(),
                            attributes,
                            properties,
                            child_node_ids: Vec::new(),
                        };
                        sub_resources.insert(id.clone(), node);
                    }
                }
            }

            Ok(GodotScene {
                attributes: scene_attributes,
                nodes,
                ext_resources,
                sub_resources,
                root_node_id,
            })
        }
        None => Err("Failed to parse scene file".to_string()),
    };
}

use automerge::{
    transaction::{Transactable, Transaction},
    ObjType, ROOT,
};
use autosurgeon::{Hydrate, Reconcile};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Parser, Query, QueryCursor};
use uuid;

use crate::doc_utils::SimpleDocReader;

#[derive(Debug, Clone)]
pub struct GodotScene {
    load_steps: i32,
    format: i32,
    uid: String,
    nodes: HashMap<String, GodotNode>,
    ext_resources: Vec<ExternalResourceNode>,
    sub_resources: HashMap<String, SubResourceNode>,
    root_node_id: String,
}

#[derive(Debug, Clone)]
enum TypeOrInstance {
    Type(String),
    Instance(String),
}

#[derive(Debug, Clone)]
pub struct GodotNode {
    id: String,
    name: String,
    type_or_instance: TypeOrInstance, // a node either has a type or an instance property
    parent: Option<String>,
    owner: Option<String>,
    index: Option<i32>,
    groups: Option<String>,
    properties: HashMap<String, String>,
    child_node_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ExternalResourceNode {
    resource_type: String,
    uid: Option<String>,
    path: String,
    id: String,
}

#[derive(Debug, Clone)]
pub struct SubResourceNode {
    id: String,
    resource_type: String,
    properties: HashMap<String, String>, // key value pairs below the section header
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

            // todo: reconcile other fields

            for (key, value) in &node.properties {
                tx.put(&node_key, key, value);
            }
        }
    }

    pub fn serialize(&self) -> String {
        let mut output = String::new();

        // Scene header
        if self.load_steps != 0 {
            output.push_str(&format!(
                "[gd_scene load_steps={} format={} uid={}]\n\n",
                self.load_steps, self.format, self.uid
            ));
        } else {
            output.push_str(&format!(
                "[gd_scene format={} uid={}]\n\n",
                self.format, self.uid
            ));
        }

        // External resources

        let mut serialized_ext_resources = HashSet::new();

        
        for resource in &self.ext_resources {   
            // the same resource could be in the list multiple times, so we need to check if we already serialized it
            // todo: think about how to properly handle this
            if serialized_ext_resources.contains(&resource.id) {
                continue;
            }

            serialized_ext_resources.insert(resource.id.clone());

            output.push_str(&format!("[ext_resource type=\"{}\"", resource.resource_type));
            if let Some(uid) = &resource.uid {
                output.push_str(&format!(" uid=\"{}\"", uid));
            }
            output.push_str(&format!(" path=\"{}\" id=\"{}\"]\n", resource.path, resource.id));
        }

        if !self.ext_resources.is_empty() {
            output.push('\n');
        }

        // Sub-resources sorted by id (a to z)
        let mut sorted_sub_resources: Vec<(&String, &SubResourceNode)> = self.sub_resources.iter().collect();
        sorted_sub_resources.sort_by(|(a,_), (b,_)| a.to_lowercase().cmp(&b.to_lowercase()));
        for (_, resource) in sorted_sub_resources {
            output.push_str(&format!("[sub_resource type=\"{}\" id=\"{}\"]\n", resource.resource_type, resource.id));    

            // Properties sorted by name (a to z)
            let mut sorted_props: Vec<(&String, &String)> = resource.properties.iter().collect();
            sorted_props.sort_by(|(a,_), (b,_)| a.to_lowercase().cmp(&b.to_lowercase()));
            for (key, value) in sorted_props {
                output.push_str(&format!("{}={}\n", key, value));
            }

            output.push('\n');
        }

        if !self.nodes.is_empty() {
            if let Some(root_node) = self.nodes.get(&self.root_node_id) {
                self.serialize_node(&mut output, root_node);
            }
        }

        output
    }

    fn serialize_node(&self, output: &mut String, node: &GodotNode) {
        output.push_str(&format!("[node name=\"{}\"", node.name));

        if let TypeOrInstance::Type(t) = &node.type_or_instance {
            output.push_str(&format!(" type=\"{}\"", t));
        }

        if let Some(parent) = &node.parent {
            output.push_str(&format!(" parent=\"{}\"", parent));
        }

        if let TypeOrInstance::Instance(i) = &node.type_or_instance {
            output.push_str(&format!(" instance={}", i));
        }

        if let Some(owner) = &node.owner {
            output.push_str(&format!(" owner=\"{}\"", owner));
        }

        if let Some(index) = &node.index {
            output.push_str(&format!(" index={}", index));
        }

        if let Some(groups) = &node.groups {
            output.push_str(&format!(" groups={}", groups));
        }

        output.push_str("]\n");

        // Properties sorted a to z
        let mut sorted_props: Vec<(&String, &String)> = node.properties.iter().collect();
        sorted_props.sort_by(|(a,_), (b,_)| a.to_lowercase().cmp(&b.to_lowercase()));
        for (key, value) in sorted_props {
            output.push_str(&format!("{}={}\n", key, value));
        }

        // Always add a blank line after a node's properties
        output.push('\n');

        // Recursively serialize children
        for child_id in &node.child_node_ids {
            if let Some(child_node) = self.nodes.get(child_id) {
                self.serialize_node(output, child_node);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SceneMetadata {
    load_steps: i32,
    format: i32,
    uid: String,
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

            // Initialize with default values
            let mut scene_metadata: Option<SceneMetadata> = None;
            let mut nodes: HashMap<String, GodotNode> = HashMap::new();
            let mut ext_resources: Vec<ExternalResourceNode> = Vec::new();
            let mut sub_resources: HashMap<String, SubResourceNode> = HashMap::new();
            let mut root_node_id: Option<String> = None;

            // Stack to track node hierarchy
            let mut ancestor_nodes: Vec<GodotNode> = Vec::new();

            for m in matches {
                let mut heading = HashMap::new();
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
                                        heading.insert(text.to_string(), value.to_string());
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

                // SCENE HEADER
                //
                if section_id == "gd_scene" {
                    // First section with ID "gd_scene" is the scene header
                    // Extract specific properties from attributes

                    let load_steps = heading
                        .get("load_steps")
                        .and_then(|ls| ls.parse::<i32>().ok())
                        .unwrap_or(0);

                    let format = match heading.get("format").and_then(|f| f.parse::<i32>().ok()) {
                        Some(format) => format,
                        None => {
                            return Err(
                                "Missing required 'format' attribute in scene header".to_string()
                            )
                        }
                    };

                    let uid = match heading.get("uid") {
                        Some(uid) => uid.clone(),
                        None => {
                            return Err(
                                "Missing required 'uid' attribute in scene header".to_string()
                            )
                        }
                    };

                    scene_metadata = Some(SceneMetadata {
                        load_steps,
                        format,
                        uid,
                    });

                // NODE
                //
                } else if section_id == "node" {
                    // Create a node and add it to the nodes map
                    let mut node_id = String::new();

                    // Check if node has a patchwork_id in metadata
                    if let Some(patchwork_id) = properties.get("metadata/patchwork_id") {
                        node_id = patchwork_id.clone();
                    } else {
                        // Generate a UUID if no patchwork_id exists
                        node_id = uuid::Uuid::new_v4().simple().to_string();
                        properties.insert(
                            "metadata/patchwork_id".to_string(),
                            format!("\"{}\"", node_id).clone(),
                        );
                    }

                    let name = match heading.get("name") {
                        Some(name) => unquote(name),
                        None => {
                            return Err(
                                "Missing required 'name' attribute in node section".to_string()
                            )
                        }
                    };

                    let type_or_instance = if let Some(type_value) = heading.get("type") {
                        TypeOrInstance::Type(unquote(&type_value))
                    } else if let Some(instance_value) = heading.get("instance") {
                        TypeOrInstance::Instance(unquote(instance_value))
                    } else {
                        return Err(format!(
                            "Missing required 'type' or 'instance' attribute in node section {}",
                            name
                        ));
                    };

                    let parent = heading.get("parent").cloned().map(|p| unquote(&p));
        
                
                    if root_node_id.is_none() {
                        root_node_id = Some(node_id.clone());

                    } else {
                        loop {
                            let ancestor = match ancestor_nodes.last_mut() {
                                Some(ancestor) => ancestor,
                                None => {
                                    return Err("parent node not found in hierarchy".to_string())
                                }
                            };

                            if Some(ancestor.name.clone()) == parent
                                || 
                                // special case, the root node is refered to by "."
                                (parent == Some(".".to_string())       
                                    && Some(ancestor.id.clone()) == root_node_id)
                            {      
                                nodes.get_mut(&ancestor.id).unwrap().child_node_ids.push(node_id.clone());
                                break;
                            } else {
                                ancestor_nodes.pop();
                            }
                        };
                    }

                    let node = GodotNode {
                        id: node_id.clone(),
                        name,
                        type_or_instance,
                        parent: parent.clone(),
                        owner: heading.get("owner").cloned().map(|o| unquote(&o)),
                        index: heading.get("index").and_then(|i| i.parse::<i32>().ok()),
                        groups: heading.get("groups").cloned(),
                        properties,
                        child_node_ids: Vec::new()
                    };

                    nodes.insert(node_id.clone(), node.clone());
                    ancestor_nodes.push(node);

                // EXTERNAL RESOURCE
                //
                } else if section_id == "ext_resource" {
                    // Add to ext_resources map

                    let resource_type = match heading.get("type").cloned() {
                        Some(resource_type) => unquote(&resource_type),
                        None => {
                            return Err("Missing required 'type' attribute in ext_resource section".to_string())
                        }
                    };

                    let uid: Option<String> = heading.get("uid").cloned().map(|uid| unquote(&uid));

                    let path = match heading.get("path").cloned() {
                        Some(path) => unquote(&path),
                        None => {
                            return Err("Missing required 'path' attribute in ext_resource section".to_string())
                        }
                    };

                    let id = match heading.get("id").cloned() {
                        Some(id) => unquote(&id),
                        None => {
                            return Err("Missing required 'id' attribute in ext_resource section".to_string())
                        }
                    };

                    ext_resources.push(ExternalResourceNode {
                        resource_type,
                        uid,
                        path,
                        id,
                    });

                // SUB-RESOURCE
                //
                } else if section_id == "sub_resource" {

                    let id = match heading.get("id").cloned() {
                        Some(id) => unquote(&id),
                        None => {
                            return Err("Missing required 'id' attribute in sub_resource section".to_string())
                        }
                    };

                    let resource_type = match heading.get("type").cloned() {
                        Some(resource_type) => unquote(&resource_type),
                        None => {
                            return Err("Missing required 'type' attribute in sub_resource section".to_string())
                        }
                    };


                    let sub_resource = SubResourceNode {
                        id: id.clone(),
                        resource_type,
                        properties,
                    };

                    sub_resources.insert(id, sub_resource);
                }
            }

            let scene_metadata = match scene_metadata {
                Some(metadata) => metadata,
                None => return Err(String::from("missing gd_scene header")),
            };

            let root_node_id = match root_node_id {
                Some(id) => id,
                None => return Err(String::from("missing root node")),
            };

            Ok(GodotScene {
                load_steps: scene_metadata.load_steps,
                format: scene_metadata.format,
                uid: scene_metadata.uid,
                nodes,
                ext_resources,
                sub_resources,
                root_node_id,
            })
        }
        None => Err("Failed to parse scene file".to_string()),
    };
}

fn unquote(string: &String) -> String {
    if string.starts_with("\"") && string.ends_with("\"") {
        string[1..string.len() - 1].to_string()
    } else {
        string.clone()
    }
}

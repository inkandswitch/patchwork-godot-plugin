use automerge::{
    transaction::{Transactable, Transaction}, Automerge, ObjType, ReadDoc, ROOT
};
use autosurgeon::{Hydrate, Reconcile};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Parser, Query, QueryCursor};
use uuid;

use crate::doc_utils::SimpleDocReader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotScene {
    pub load_steps: i64,
    pub format: i64,
    pub uid: String,
    pub root_node_id: String,
    pub ext_resources: Vec<ExternalResourceNode>,
    pub sub_resources: HashMap<String, SubResourceNode>,
    pub nodes: HashMap<String, GodotNode>,
    pub connections: Vec<GodotConnection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeOrInstance {
    Type(String),
    Instance(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotNode {
    pub id: String,
    pub name: String,
    pub type_or_instance: TypeOrInstance, // a node either has a type or an instance property
    pub parent: Option<String>,
    pub owner: Option<String>,
    pub index: Option<i32>,
    pub groups: Option<String>,
    pub properties: HashMap<String, String>,
    pub child_node_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotConnection {
    pub signal: String,
    pub from: String,
    pub to: String,
    pub method: String,
    pub flags: Option<i32>,
    pub unbinds: Option<i32>,
    pub binds: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalResourceNode {
    pub resource_type: String,
    pub uid: Option<String>,
    pub path: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubResourceNode {
    pub id: String,
    pub resource_type: String,
    pub properties: HashMap<String, String>, // key value pairs below the section header
}

impl GodotScene {
    pub fn reconcile(&self, tx: &mut Transaction, path: String) {
        let files = tx
            .get_obj_id(ROOT, "files")
            .unwrap_or_else(|| tx.put_object(ROOT, "files", ObjType::Map).unwrap());

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


        // Store Scene Metadata
        tx.put(&scene_file, "uid", self.uid.clone()).unwrap();
        tx.put(&scene_file, "load_steps", self.load_steps).unwrap();
        tx.put(&scene_file, "format", self.format).unwrap();

        // Store root node id
        tx.put(&structured_content, "root_node_id", self.root_node_id.clone()).unwrap();

        // Reconcile nodes
        for (node_id, node) in &self.nodes {
            let node_obj = tx
                .get_obj_id(&nodes, node_id)
                .unwrap_or_else(|| tx.put_object(&nodes, node_id, ObjType::Map).unwrap());
            
            // Store basic node properties
            tx.put(&node_obj, "id", node.id.clone()).unwrap();
            tx.put(&node_obj, "name", node.name.clone()).unwrap();
            
            // Store type or instance
            match &node.type_or_instance {
                TypeOrInstance::Type(type_name) => {
                    tx.put(&node_obj, "type", type_name.clone()).unwrap();
                    // Remove instance if it exists
                    if tx.get_string(&node_obj, "instance").is_some() {
                        tx.delete(&node_obj, "instance").unwrap();
                    }
                },
                TypeOrInstance::Instance(instance_id) => {
                    tx.put(&node_obj, "instance", instance_id.clone()).unwrap();
                    // Remove type if it exists
                    if tx.get_string(&node_obj, "type").is_some() {
                        tx.delete(&node_obj, "type").unwrap();
                    }
                }
            }
            
            // Store optional properties
            if let Some(parent) = &node.parent {
                tx.put(&node_obj, "parent", parent.clone()).unwrap();
            } else if tx.get_string(&node_obj, "parent").is_some() {
                tx.delete(&node_obj, "parent").unwrap();
            }
            
            if let Some(owner) = &node.owner {
                tx.put(&node_obj, "owner", owner.clone()).unwrap();
            } else if tx.get_string(&node_obj, "owner").is_some() {
                tx.delete(&node_obj, "owner").unwrap();
            }
            
            if let Some(index) = node.index {
                tx.put(&node_obj, "index", index as i64).unwrap();
            } else if tx.get_int(&node_obj, "index").is_some() {
                tx.delete(&node_obj, "index").unwrap();
            }
            
            if let Some(groups) = &node.groups {
                tx.put(&node_obj, "groups", groups.clone()).unwrap();
            } else if tx.get_string(&node_obj, "groups").is_some() {
                tx.delete(&node_obj, "groups").unwrap();
            }
            
            // Store properties
            let properties_obj = tx
                .get_obj_id(&node_obj, "properties")
                .unwrap_or_else(|| tx.put_object(&node_obj, "properties", ObjType::Map).unwrap());
            
            // Get existing properties to check for deletions
            let mut existing_props = tx.keys(&properties_obj).collect::<HashSet<_>>();
            
            // Add or update properties
            for (key, value) in &node.properties {
                tx.put(&properties_obj, key, value.clone()).unwrap();
                existing_props.remove(key);
            }
            
            // Remove properties that no longer exist
            for key in existing_props {
                tx.delete(&properties_obj, &key).unwrap();
            }
            
            // Store child node IDs
            let children_obj = tx
                .get_obj_id(&node_obj, "child_node_ids")
                .unwrap_or_else(|| tx.put_object(&node_obj, "child_node_ids", ObjType::List).unwrap());
            


            // reconcile child node ids
            for (i, new_child_node_id) in node.child_node_ids.iter().enumerate() {
                if let Some(current_child_node_id) = tx.get_string(&children_obj, i) {
                    if current_child_node_id != *new_child_node_id {
                        tx.put(&children_obj, i, new_child_node_id.clone()).unwrap();
                    }
                } else {
                    let _ = tx.insert(&children_obj, i, new_child_node_id.clone());
                    println!("new_child_node_id: {}", new_child_node_id);

                }
            }


            // // delete child node ids if they are not in the node

            // let new_child_node_ids_count = node.child_node_ids.len();
            // let prev_child_node_ids_count = tx.length(&children_obj);

            // if new_child_node_ids_count < prev_child_node_ids_count {
            //     for i in (new_child_node_ids_count..prev_child_node_ids_count).rev() {
            //         tx.delete(&children_obj, i).unwrap();
            //     }
            // }
        }
        
        // Remove nodes that are in the document but not in the scene
        let existing_nodes =  tx.keys(&nodes).collect::<HashSet<_>>();    
        for node_id in existing_nodes {
            if !self.nodes.contains_key(&node_id) {
                tx.delete(&nodes, &node_id).unwrap();
            }
        }
    }

    pub fn hydrate(doc: &mut Automerge, path: &str) -> Result<Self, String> {
        // Get the files object
        let files = doc.get_obj_id(ROOT, "files")
            .ok_or_else(|| "Could not find files object in document".to_string())?;

        // Get the specific file at the given path
        let scene_file = doc.get_obj_id(&files, path)
            .ok_or_else(|| format!("Could not find file at path: {}", path))?;

        // Get the structured content
        let structured_content = doc.get_obj_id(&scene_file, "structured_content")
            .ok_or_else(|| "Could not find structured_content in file".to_string())?;

        // Get the uid
        let uid = doc.get_string(&scene_file, "uid")
            .ok_or_else(|| "Could not find uid in scene_file".to_string())?;

        let load_steps = doc.get_int(&scene_file, "load_steps")
            .ok_or_else(|| "Could not find load_steps in scene_file".to_string())?;

        let format = doc.get_int(&scene_file, "format")
            .ok_or_else(|| "Could not find format in scene_file".to_string())?;

        // Get the nodes object
        let nodes_obj = doc.get_obj_id(&structured_content, "nodes")
            .ok_or_else(|| "Could not find nodes in structured_content".to_string())?;

        let root_node_id = doc.get_string(&structured_content, "root_node_id")
            .ok_or_else(|| "Could not find root_node_id in structured_content".to_string())?;

        // Create a map to store the nodes
        let mut nodes = HashMap::new();

        // Iterate through all node IDs in the nodes object
        for node_id in doc.keys(&nodes_obj) {
            // Get the node object
            let node_obj = doc.get_obj_id(&nodes_obj, &node_id)
                .ok_or_else(|| format!("Could not find node object for ID: {}", node_id))?;

            // Extract node properties
            let id = doc.get_string(&node_obj, "id").unwrap_or_else(|| node_id.clone());
            let name = doc.get_string(&node_obj, "name")
                .ok_or_else(|| format!("Node {} is missing required name property", node_id))?;

            // Determine if this is a type or instance
            let type_or_instance = if let Some(type_name) = doc.get_string(&node_obj, "type") {
                TypeOrInstance::Type(type_name)
            } else if let Some(instance_id) = doc.get_string(&node_obj, "instance") {
                TypeOrInstance::Instance(instance_id)
            } else {
                return Err(format!("Node {} is missing both type and instance properties", node_id));
            };

            // Get optional properties
            let parent = doc.get_string(&node_obj, "parent");
            let owner = doc.get_string(&node_obj, "owner");
            let index = doc.get_int(&node_obj, "index").map(|i| i as i32);
            let groups = doc.get_string(&node_obj, "groups");


            // Get node properties
            let properties_obj = doc.get_obj_id(&node_obj, "properties")
            .ok_or_else(|| format!("Could not find properties object for node: {}", node_id))?;
            let mut properties = HashMap::new();
            for key in doc.keys(&properties_obj) {
                let value = doc.get_string(&properties_obj, &key)
                    .ok_or_else(|| format!("Could not find value for property: {}", key))?;

                properties.insert(key, value);
            }

            // Get child node IDs
            let mut child_node_ids = Vec::new();
            if let Some(children_obj) = doc.get_obj_id(&node_obj, "child_node_ids") {
                let length = doc.length(&children_obj);
                for i in 0..length {
                    if let Some(child_id) = doc.get_string(&children_obj, i) {
                        child_node_ids.push(child_id);
                    }
                }
            }

            // Create the node
            let node = GodotNode {
                id,
                name,
                type_or_instance,
                parent,
                owner,
                index,
                groups,
                properties,
                child_node_ids,
            };

            // Add the node to our map
            nodes.insert(node_id, node);
        }

        if nodes.is_empty() {
            return Err("Scene contains no nodes".to_string());
        }

        // Create a GodotScene with default values for everything except nodes
        Ok(GodotScene {
            load_steps,
            format,
            uid,
            root_node_id,
            ext_resources: Vec::new(),
            sub_resources: HashMap::new(),
            nodes,
            connections: Vec::new(),
        })
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

        for connection in &self.connections {
            output.push_str(&format!("[connection signal=\"{}\" from=\"{}\" to=\"{}\" method=\"{}\"", connection.signal, connection.from, connection.to, connection.method));
            if let Some(flags) = connection.flags {
                output.push_str(&format!(" flags={}", flags));
               }
            if let Some(unbinds) = connection.unbinds {
                output.push_str(&format!(" unbinds={}", unbinds));
            }
            if let Some(binds) = &connection.binds {
                output.push_str(&format!(" binds={}", binds));
            }
            output.push_str("]\n");
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
    pub load_steps: i64,
    pub format: i64,
    pub uid: String,
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
            let mut connections: Vec<GodotConnection> = Vec::new();
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
                        .and_then(|ls| ls.parse::<i64>().ok())
                        .unwrap_or(0);

                    let format = match heading.get("format").and_then(|f| f.parse::<i64>().ok()) {
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

                // CONNECTION
                //
                } else if section_id == "connection" {
                    let signal = match heading.get("signal").cloned() {
                        Some(signal) => unquote(&signal),
                        None => {
                            return Err("Missing required 'signal' attribute in connection section".to_string())
                        }
                    };

                    let from = match heading.get("from").cloned() {
                        Some(from) => unquote(&from),
                        None => {
                            return Err("Missing required 'from' attribute in connection section".to_string())
                        }
                    };

                    let to = match heading.get("to").cloned() {
                        Some(to) => unquote(&to),
                        None => {
                            return Err("Missing required 'to' attribute in connection section".to_string())
                        }
                    }; 

                    let method = match heading.get("method").cloned() {
                        Some(method) => unquote(&method),
                        None => {
                            return Err("Missing required 'method' attribute in connection section".to_string())
                        }
                    };

                    let flags = heading.get("flags").and_then(|f| f.parse::<i32>().ok());

                    let unbinds = heading.get("unbinds").and_then(|u| u.parse::<i32>().ok());

                    let binds =  heading.get("binds").cloned().map(|b| unquote(&b));

                    
                    let connection = GodotConnection {
                        signal,
                        from,
                        to,
                        method,
                        flags,
                        unbinds,
                        binds,
                    };
                
                    connections.push(connection);                                
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
                root_node_id,
                ext_resources,
                sub_resources,
                nodes,
                connections,
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

use automerge::{
    transaction::{Transactable, Transaction},
    Automerge, ObjType, ReadDoc, ROOT,
};
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
    pub ext_resources: HashMap<String, ExternalResourceNode>,
    pub sub_resources: HashMap<String, SubResourceNode>,
    pub nodes: HashMap<String, GodotNode>,
    pub connections: HashMap<String, GodotConnection>, // key is concatenation of all properties of the connection
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
    pub parent_id: Option<String>,
    pub owner: Option<String>,
    pub index: Option<i64>,
    pub groups: Option<String>,
    pub properties: HashMap<String, String>,
    pub child_node_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotConnection {
    pub signal: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub method: String,
    pub flags: Option<i64>,
    pub unbinds: Option<i64>,
    pub binds: Option<String>,
}

impl GodotConnection {
    pub fn id(&self) -> String {
        format!(
            "{}-{}-{}-{}-{}-{}-{}",
            self.signal,
            self.from_node_id,
            self.to_node_id,
            self.method,
            self.flags.map_or("".to_string(), |flags| flags.to_string()),
            self.unbinds
                .map_or("".to_string(), |unbinds| unbinds.to_string()),
            self.binds
                .clone()
                .map_or("[]".to_string(), |binds| binds.to_string())
        )
    }
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
    pub fn get_node_path(&self, node_id: &str) -> String {
        let mut path = String::new();
        let mut current_id = node_id;

        loop {
            let node = self.nodes.get(current_id).unwrap();

            path = if path.is_empty() {
                node.name.clone()
            } else {
                format!("{}/{}", node.name, path)
            };

            match &node.parent_id {
                Some(parent_id) => {
                    current_id = parent_id.as_str();

                    if current_id == self.root_node_id {
                        return path;
                    }
                }
                None => {
                    return path;
                }
            }
        }
    }

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

        let connections = tx
            .get_obj_id(&scene_file, "connections")
            .unwrap_or_else(|| {
                tx.put_object(&scene_file, "connections", ObjType::Map)
                    .unwrap()
            });

        // Store Scene Metadata
        tx.put(&scene_file, "uid", self.uid.clone()).unwrap();
        tx.put(&scene_file, "load_steps", self.load_steps).unwrap();
        tx.put(&scene_file, "format", self.format).unwrap();

        // Store root node id
        tx.put(
            &structured_content,
            "root_node_id",
            self.root_node_id.clone(),
        )
        .unwrap();

        // Reconcile external resources
        let ext_resources = tx
            .get_obj_id(&scene_file, "ext_resources")
            .unwrap_or_else(|| {
                tx.put_object(&scene_file, "ext_resources", ObjType::Map)
                    .unwrap()
            });

        for (id, resource) in &self.ext_resources {
            let resource_obj = tx
                .get_obj_id(&ext_resources, id)
                .unwrap_or_else(|| tx.put_object(&ext_resources, id, ObjType::Map).unwrap());

            tx.put(
                &resource_obj,
                "resource_type",
                resource.resource_type.clone(),
            )
            .unwrap();

            if let Some(uid) = &resource.uid {
                tx.put(&resource_obj, "uid", uid.clone()).unwrap();
            } else if tx.get_string(&resource_obj, "uid").is_some() {
                tx.delete(&resource_obj, "uid").unwrap();
            }

            tx.put(&resource_obj, "path", resource.path.clone())
                .unwrap();
            tx.put(&resource_obj, "id", resource.id.clone()).unwrap();
        }

        // Remove external resources that are not in the scene
        let existing_resource_ids = tx.keys(&ext_resources).collect::<HashSet<_>>();
        for resource_id in existing_resource_ids {
            if !self.ext_resources.contains_key(&resource_id) {
                tx.delete(&ext_resources, &resource_id).unwrap();
            }
        }

        // Reconcile sub resources
        let sub_resources = tx
            .get_obj_id(&scene_file, "sub_resources")
            .unwrap_or_else(|| {
                tx.put_object(&scene_file, "sub_resources", ObjType::Map)
                    .unwrap()
            });

        for (id, resource) in &self.sub_resources {
            let resource_obj = tx
                .get_obj_id(&sub_resources, id)
                .unwrap_or_else(|| tx.put_object(&sub_resources, id, ObjType::Map).unwrap());

            tx.put(
                &resource_obj,
                "resource_type",
                resource.resource_type.clone(),
            )
            .unwrap();

            tx.put(&resource_obj, "id", resource.id.clone()).unwrap();

            let properties_obj = tx
                .get_obj_id(&resource_obj, "properties")
                .unwrap_or_else(|| {
                    tx.put_object(&resource_obj, "properties", ObjType::Map)
                        .unwrap()
                });

            let mut existing_props = tx.keys(&properties_obj).collect::<HashSet<_>>();

            // Add or update properties
            for (key, value) in &resource.properties {
                if let Some(existing_value) = tx.get_string(&properties_obj, key) {
                    if existing_value != *value {
                        tx.put(&properties_obj, key, value.clone()).unwrap();
                    }
                } else {
                    tx.put(&properties_obj, key, value.clone()).unwrap();
                }
                existing_props.remove(key);
            }

            // Remove properties that no longer exist
            for key in existing_props {
                tx.delete(&properties_obj, &key).unwrap();
            }
        }

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
                }
                TypeOrInstance::Instance(instance_id) => {
                    tx.put(&node_obj, "instance", instance_id.clone()).unwrap();
                    // Remove type if it exists
                    if tx.get_string(&node_obj, "type").is_some() {
                        tx.delete(&node_obj, "type").unwrap();
                    }
                }
            }

            // Store optional properties
            if let Some(parent_id) = &node.parent_id {
                tx.put(&node_obj, "parent_id", parent_id.clone()).unwrap();
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
            let properties_obj = tx.get_obj_id(&node_obj, "properties").unwrap_or_else(|| {
                tx.put_object(&node_obj, "properties", ObjType::Map)
                    .unwrap()
            });

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
                .unwrap_or_else(|| {
                    tx.put_object(&node_obj, "child_node_ids", ObjType::List)
                        .unwrap()
                });

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
        }

        // Remove nodes that are in the document but not in the scene
        let existing_nodes = tx.keys(&nodes).collect::<Vec<_>>();
        for node_id in existing_nodes {
            if !self.nodes.contains_key(&node_id) {
                tx.delete(&nodes, &node_id).unwrap();
            }
        }

        // Reconcile connections
        for (id, connection) in self.connections.iter() {
            let connection_obj = tx.get_obj_id(&connections, id);

            // only need to create the connection object if it doesn't exist
            if connection_obj.is_none() {
                let connection_obj = tx.put_object(&connections, id, ObjType::Map).unwrap();

                tx.put(&connection_obj, "signal", connection.signal.clone())
                    .unwrap();
                tx.put(
                    &connection_obj,
                    "from_node_id",
                    connection.from_node_id.clone(),
                )
                .unwrap();
                tx.put(&connection_obj, "to_node_id", connection.to_node_id.clone())
                    .unwrap();
                tx.put(&connection_obj, "method", connection.method.clone())
                    .unwrap();

                if let Some(flags) = connection.flags {
                    tx.put(&connection_obj, "flags", flags).unwrap();
                }
                if let Some(unbinds) = connection.unbinds {
                    tx.put(&connection_obj, "unbinds", unbinds).unwrap();
                }
                if let Some(binds) = &connection.binds {
                    tx.put(&connection_obj, "binds", binds.clone()).unwrap();
                }
            }
        }

        // Remove connections that are in the document but not in the scene
        for connection_id in tx.keys(&connections).collect::<Vec<_>>() {
            if !self.connections.contains_key(&connection_id) {
                let _ = tx.delete(&connections, &connection_id).unwrap();
            }
        }
    }

    pub fn hydrate(doc: &mut Automerge, path: &str) -> Result<Self, String> {
        // Get the files object
        let files = doc
            .get_obj_id(ROOT, "files")
            .ok_or_else(|| "Could not find files object in document".to_string())?;

        // Get the specific file at the given path
        let scene_file = doc
            .get_obj_id(&files, path)
            .ok_or_else(|| format!("Could not find file at path: {}", path))?;

        // Get the structured content
        let structured_content = doc
            .get_obj_id(&scene_file, "structured_content")
            .ok_or_else(|| "Could not find structured_content in file".to_string())?;

        // Get the uid
        let uid = doc
            .get_string(&scene_file, "uid")
            .ok_or_else(|| "Could not find uid in scene_file".to_string())?;

        let load_steps = doc
            .get_int(&scene_file, "load_steps")
            .ok_or_else(|| "Could not find load_steps in scene_file".to_string())?;

        let format = doc
            .get_int(&scene_file, "format")
            .ok_or_else(|| "Could not find format in scene_file".to_string())?;

        // Get the nodes object
        let nodes_id = doc
            .get_obj_id(&structured_content, "nodes")
            .ok_or_else(|| "Could not find nodes in structured_content".to_string())?;

        let root_node_id = doc
            .get_string(&structured_content, "root_node_id")
            .ok_or_else(|| "Could not find root_node_id in structured_content".to_string())?;

        // Create a map to store the nodes

        // Iterate through all external resources

        let mut ext_resources = HashMap::new();

        let ext_resources_id = doc
            .get_obj_id(&scene_file, "ext_resources")
            .ok_or_else(|| "Could not find ext_resources in scene_file".to_string())?;

        for resource_id in doc.keys(&ext_resources_id) {
            let resource_obj = doc
                .get_obj_id(&ext_resources_id, &resource_id)
                .ok_or_else(|| format!("Could not find resource object for ID: {}", resource_id))?;

            let resource_type = doc
                .get_string(&resource_obj, "resource_type")
                .ok_or_else(|| format!("Could not find resource_type for ID: {}", resource_id))?;

            let path = doc
                .get_string(&resource_obj, "path")
                .ok_or_else(|| format!("Could not find path for ID: {}", resource_id))?;

            let id = doc
                .get_string(&resource_obj, "id")
                .ok_or_else(|| format!("Could not find id for ID: {}", resource_id))?;

            let uid = doc.get_string(&resource_obj, "uid");

            let external_resource = ExternalResourceNode {
                resource_type,
                uid,
                path,
                id,
            };

            ext_resources.insert(resource_id.clone(), external_resource);
        }

        // Itereate through all sub resources
        let mut sub_resources = HashMap::new();

        let sub_resources_id = doc
            .get_obj_id(&scene_file, "sub_resources")
            .ok_or_else(|| "Could not find sub_resources in scene_file".to_string())?;

        for sub_resource_id in doc.keys(&sub_resources_id) {
            let sub_resource_obj = doc
                .get_obj_id(&sub_resources_id, &sub_resource_id)
                .ok_or_else(|| {
                    format!(
                        "Could not find sub_resource object for ID: {}",
                        sub_resource_id
                    )
                })?;

            let resource_type = doc
                .get_string(&sub_resource_obj, "resource_type")
                .ok_or_else(|| {
                    format!("Could not find resource_type for ID: {}", sub_resource_id)
                })?;

            let id = doc
                .get_string(&sub_resource_obj, "id")
                .ok_or_else(|| format!("Could not find id for ID: {}", sub_resource_id))?;

            let properties_obj =
                doc.get_obj_id(&sub_resource_obj, "properties")
                    .ok_or_else(|| {
                        format!(
                            "Could not find properties object for ID: {}",
                            sub_resource_id
                        )
                    })?;

            let mut properties = HashMap::new();
            for key in doc.keys(&properties_obj) {
                let value = doc
                    .get_string(&properties_obj, &key)
                    .ok_or_else(|| format!("Could not find value for property: {}", key))?;

                properties.insert(key, value);
            }

            let sub_resource = SubResourceNode {
                id,
                resource_type,
                properties,
            };

            sub_resources.insert(sub_resource_id.clone(), sub_resource);
        }

        // Iterate through all node IDs in the nodes object

        let mut nodes = HashMap::new();

        for node_id in doc.keys(&nodes_id) {
            // Get the node object
            let node_obj = doc
                .get_obj_id(&nodes_id, &node_id)
                .ok_or_else(|| format!("Could not find node object for ID: {}", node_id))?;

            // Extract node properties
            let id = doc
                .get_string(&node_obj, "id")
                .unwrap_or_else(|| node_id.clone());
            let name = doc
                .get_string(&node_obj, "name")
                .ok_or_else(|| format!("Node {} is missing required name property", node_id))?;

            // Determine if this is a type or instance
            let type_or_instance = if let Some(type_name) = doc.get_string(&node_obj, "type") {
                TypeOrInstance::Type(type_name)
            } else if let Some(instance_id) = doc.get_string(&node_obj, "instance") {
                TypeOrInstance::Instance(instance_id)
            } else {
                return Err(format!(
                    "Node {} is missing both type and instance properties",
                    node_id
                ));
            };

            // Get optional properties
            let parent_id = doc.get_string(&node_obj, "parent_id");
            let owner = doc.get_string(&node_obj, "owner");
            let index = doc.get_int(&node_obj, "index").map(|i| i);
            let groups = doc.get_string(&node_obj, "groups");

            // Get node properties
            let properties_obj = doc
                .get_obj_id(&node_obj, "properties")
                .ok_or_else(|| format!("Could not find properties object for node: {}", node_id))?;
            let mut properties = HashMap::new();
            for key in doc.keys(&properties_obj) {
                let value = doc
                    .get_string(&properties_obj, &key)
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
                parent_id,
                owner,
                index,
                groups,
                properties,
                child_node_ids,
            };

            // Add the node to our map
            nodes.insert(node_id, node);
        }

        // Iterate through all connections
        let mut connections = HashMap::new();

        let connections_id = doc
            .get_obj_id(&scene_file, "connections")
            .ok_or_else(|| "Could not find connections in scene document".to_string())?;

        for connection_id in doc.keys(&connections_id) {
            let connection_obj =
                doc.get_obj_id(&connections_id, &connection_id)
                    .ok_or_else(|| {
                        format!("Could not find connection object for ID: {}", connection_id)
                    })?;

            let signal = doc.get_string(&connection_obj, "signal").ok_or_else(|| {
                format!("Could not find signal for connection: {}", connection_id)
            })?;

            let from_node_id =
                doc.get_string(&connection_obj, "from_node_id")
                    .ok_or_else(|| {
                        format!(
                            "Could not find from_node_id for connection: {}",
                            connection_id
                        )
                    })?;

            let to_node_id = doc
                .get_string(&connection_obj, "to_node_id")
                .ok_or_else(|| {
                    format!(
                        "Could not find to_node_id for connection: {}",
                        connection_id
                    )
                })?;

            let method = doc.get_string(&connection_obj, "method").ok_or_else(|| {
                format!("Could not find method for connection: {}", connection_id)
            })?;

            let flags = doc.get_int(&connection_obj, "flags");

            let unbinds = doc.get_int(&connection_obj, "unbinds");

            let binds = doc.get_string(&connection_obj, "binds");

            let connection = GodotConnection {
                signal,
                from_node_id,
                to_node_id,
                method,
                flags,
                unbinds,
                binds,
            };

            connections.insert(connection_id.clone(), connection);
        }

        // Create a GodotScene with default values for everything except nodes
        Ok(GodotScene {
            load_steps,
            format,
            uid,
            root_node_id,
            ext_resources,
            sub_resources,
            nodes,
            connections,
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

        // sort resources by id (a to z)
        let mut sorted_ext_resources: Vec<(&String, &ExternalResourceNode)> =
            self.ext_resources.iter().collect();
        sorted_ext_resources.sort_by(|(a, _), (b, _)| a.to_lowercase().cmp(&b.to_lowercase()));

        for (_, resource) in sorted_ext_resources {
            output.push_str(&format!(
                "[ext_resource type=\"{}\"",
                resource.resource_type
            ));
            if let Some(uid) = &resource.uid {
                output.push_str(&format!(" uid=\"{}\"", uid));
            }
            output.push_str(&format!(
                " path=\"{}\" id=\"{}\"]\n",
                resource.path, resource.id
            ));
        }

        if !self.ext_resources.is_empty() {
            output.push('\n');
        }

        // Sub-resources sorted by id (a to z)
        let mut sorted_sub_resources: Vec<(&String, &SubResourceNode)> =
            self.sub_resources.iter().collect();
        sorted_sub_resources.sort_by(|(a, _), (b, _)| a.to_lowercase().cmp(&b.to_lowercase()));
        for (_, resource) in sorted_sub_resources {
            output.push_str(&format!(
                "[sub_resource type=\"{}\" id=\"{}\"]\n",
                resource.resource_type, resource.id
            ));

            // Properties sorted by name (a to z)
            let mut sorted_props: Vec<(&String, &String)> = resource.properties.iter().collect();
            sorted_props.sort_by(|(a, _), (b, _)| a.to_lowercase().cmp(&b.to_lowercase()));
            for (key, value) in sorted_props {
                output.push_str(&format!("{} = {}\n", key, value));
            }

            output.push('\n');
        }

        if !self.nodes.is_empty() {
            if let Some(root_node) = self.nodes.get(&self.root_node_id) {
                self.serialize_node(&mut output, root_node);
            }
        }

        let mut connections: Vec<(&String, &GodotConnection)> =
            self.connections.iter().collect::<Vec<_>>();

        connections.sort_by(|(id_a, _), (id_b, _)| id_a.cmp(id_b));

        for (_, connection) in connections {
            let from_path = self.get_node_path(&connection.from_node_id);
            let to_path = self.get_node_path(&connection.to_node_id);

            output.push_str(&format!(
                "[connection signal=\"{}\" from=\"{}\" to=\"{}\" method=\"{}\"",
                connection.signal, from_path, to_path, connection.method
            ));
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

        if let Some(parent_id) = &node.parent_id {
            let parent_name = if *parent_id == self.root_node_id {
                ".".to_string()
            } else {
                self.get_node_path(parent_id)
            };

            output.push_str(&format!(" parent=\"{}\"", parent_name));
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
        sorted_props.sort_by(|(a, _), (b, _)| a.to_lowercase().cmp(&b.to_lowercase()));
        for (key, value) in sorted_props {
            output.push_str(&format!("{} = {}\n", key, value));
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

    let mut parsed_node_ids = HashSet::new();

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
            let mut ext_resources: HashMap<String, ExternalResourceNode> = HashMap::new();
            let mut sub_resources: HashMap<String, SubResourceNode> = HashMap::new();
            let mut connections: HashMap<String, GodotConnection> = HashMap::new();
            let mut root_node_id: Option<String> = None;

            // Create an index to map node paths to node ids
            let mut node_id_by_node_path: HashMap<String, String> = HashMap::new();

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
                    let node_id;

                    // Check if node has a patchwork_id in metadata
                    if let Some(patchwork_id) = properties.get("metadata/patchwork_id") {
                        let patchwork_id = unquote(&patchwork_id);

                        // generate a new id if the patchwork_id is already used by another node
                        // this can happen if a node is copied and pasted in Godot
                        if parsed_node_ids.contains(&patchwork_id) {
                            node_id = uuid::Uuid::new_v4().simple().to_string();
                        } else {
                            node_id = patchwork_id;
                        }
                    } else {
                        // Generate a UUID if no patchwork_id exists
                        node_id = uuid::Uuid::new_v4().simple().to_string();
                        properties.insert(
                            "metadata/patchwork_id".to_string(),
                            format!("\"{}\"", node_id).clone(),
                        );
                    }

                    parsed_node_ids.insert(node_id.clone());

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

                    let parent_path = heading.get("parent").cloned().map(|p| unquote(&p));
                    let parent_id = match parent_path {
                        Some(parent_path) => {
                            if parent_path == "." {
                                node_id_by_node_path.insert(name.clone(), node_id.clone());
                            } else {
                                node_id_by_node_path
                                    .insert(format!("{}/{}", parent_path, name), node_id.clone());
                            }

                            match node_id_by_node_path.get(&parent_path) {
                                Some(parent_id) => {
                                    nodes
                                        .get_mut(parent_id)
                                        .unwrap()
                                        .child_node_ids
                                        .push(node_id.clone());

                                    Some(parent_id.clone())
                                }
                                None => {
                                    return Err(format!(
                                        "Can't find parent node \"{}\" for node \"{}\"",
                                        parent_path, name
                                    ))
                                }
                            }
                        }
                        None => {
                            root_node_id = Some(node_id.clone());
                            node_id_by_node_path.insert(".".to_string(), node_id.clone());
                            None
                        }
                    };

                    let node = GodotNode {
                        id: node_id.clone(),
                        name,
                        type_or_instance,
                        parent_id,
                        owner: heading.get("owner").cloned().map(|o| unquote(&o)),
                        index: heading.get("index").and_then(|i| i.parse::<i64>().ok()),
                        groups: heading.get("groups").cloned(),
                        properties,
                        child_node_ids: Vec::new(),
                    };

                    nodes.insert(node_id.clone(), node.clone());

                // EXTERNAL RESOURCE
                //
                } else if section_id == "ext_resource" {
                    // Add to ext_resources map

                    let resource_type = match heading.get("type").cloned() {
                        Some(resource_type) => unquote(&resource_type),
                        None => {
                            return Err("Missing required 'type' attribute in ext_resource section"
                                .to_string())
                        }
                    };

                    let uid: Option<String> = heading.get("uid").cloned().map(|uid| unquote(&uid));

                    let path = match heading.get("path").cloned() {
                        Some(path) => unquote(&path),
                        None => {
                            return Err("Missing required 'path' attribute in ext_resource section"
                                .to_string())
                        }
                    };

                    let id = match heading.get("id").cloned() {
                        Some(id) => unquote(&id),
                        None => {
                            return Err("Missing required 'id' attribute in ext_resource section"
                                .to_string())
                        }
                    };

                    ext_resources.insert(
                        id.clone(),
                        ExternalResourceNode {
                            resource_type,
                            uid,
                            path,
                            id,
                        },
                    );

                // SUB-RESOURCE
                //
                } else if section_id == "sub_resource" {
                    let id = match heading.get("id").cloned() {
                        Some(id) => unquote(&id),
                        None => {
                            return Err("Missing required 'id' attribute in sub_resource section"
                                .to_string())
                        }
                    };

                    let resource_type = match heading.get("type").cloned() {
                        Some(resource_type) => unquote(&resource_type),
                        None => {
                            return Err("Missing required 'type' attribute in sub_resource section"
                                .to_string())
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
                            return Err("Missing required 'signal' attribute in connection section"
                                .to_string())
                        }
                    };

                    let from_path = match heading.get("from").cloned() {
                        Some(from) => unquote(&from),
                        None => {
                            return Err("Missing required 'from' attribute in connection section"
                                .to_string())
                        }
                    };

                    let from_node_id = match node_id_by_node_path.get(&from_path) {
                        Some(node_id) => node_id.clone(),
                        None => {
                            return Err(format!(
                                "Can't find node \"{}\", {:?}",
                                from_path, node_id_by_node_path
                            ))
                        }
                    };

                    let to_path = match heading.get("to").cloned() {
                        Some(to) => unquote(&to),
                        None => {
                            return Err(
                                "Missing required 'to' attribute in connection section".to_string()
                            )
                        }
                    };

                    let to_node_id = match node_id_by_node_path.get(&to_path) {
                        Some(node_id) => node_id.clone(),
                        None => return Err(format!("Can't find node \"{}\"", from_path)),
                    };

                    let method = match heading.get("method").cloned() {
                        Some(method) => unquote(&method),
                        None => {
                            return Err("Missing required 'method' attribute in connection section"
                                .to_string())
                        }
                    };

                    let flags = heading.get("flags").and_then(|f| f.parse::<i64>().ok());

                    let unbinds = heading.get("unbinds").and_then(|u| u.parse::<i64>().ok());

                    let binds = heading.get("binds").cloned().map(|b| unquote(&b));

                    let connection = GodotConnection {
                        signal,
                        from_node_id,
                        to_node_id,
                        method,
                        flags,
                        unbinds,
                        binds,
                    };

                    connections.insert(connection.id().clone(), connection);
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

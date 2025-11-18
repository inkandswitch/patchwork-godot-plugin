use automerge::{
    transaction::{Transactable, Transaction},
    Automerge, ChangeHash, ObjType, ReadDoc, ROOT,
};
use rand::Rng;
use safer_ffi::layout::into_raw;
use std::{collections::{HashMap, HashSet}, fmt::Display};
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};
use uuid;
use crate::{doc_utils::SimpleDocReader, utils::print_doc};

const NO_NODE_UNIQUE_ID_PREFIX: &str = "<XXX>";
const UNIQUE_SCENE_ID_UNASSIGNED: i32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotScene {
    pub load_steps: i64,
    pub format: i64,
    pub uid: String,
    pub script_class: Option<String>,
    pub resource_type: String,
    pub root_node_id: Option<i32>,
    pub ext_resources: HashMap<String, ExternalResourceNode>,
    pub sub_resources: HashMap<String, SubResourceNode>,
    pub nodes: HashMap<i32, GodotNode>,
    pub connections: HashMap<String, GodotConnection>, // key is concatenation of all properties of the connection
    pub editable_instances: HashSet<String>,
    pub main_resource: Option<SubResourceNode>,
	 // TODO: this is a hack to force the frontend to resave the scene
	 // if we add a new node id to a node in the scene, it's not serialized
	 // or saved to the doc
	pub requires_resave: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeOrInstance {
    Type(String),
    Instance(String),
}

impl Display for TypeOrInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeOrInstance::Type(type_name) => write!(f, "{}", type_name),
            TypeOrInstance::Instance(instance_id) => write!(f, "{}", instance_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedProperty {
    pub value: String,
    pub order: i64,
}

impl OrderedProperty {
    pub fn new(value: String, order: i64) -> Self {
        Self { value, order }
    }
	pub fn get_value(&self) -> String {
		self.value.clone()
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotNode {
    pub id: i32,
    pub name: String,
    pub type_or_instance: TypeOrInstance, // a node either has a type or an instance property
	pub instance_placeholder: Option<String>,
    pub parent_id: Option<i32>,
	pub parent_id_path: Option<Vec<i32>>,
    pub owner: Option<String>,
	pub owner_uid_path: Option<Vec<i32>>,
    pub index: Option<i64>,
    pub groups: Option<String>,
    pub node_paths: Option<String>,
    pub properties: HashMap<String, OrderedProperty>,

    // in the automerge doc the child_node_ids are stored as a map with the key being the child node id and the value being a number that should be used for sort order
    // this allows us to reconcile the children as a set and preserve the order to some extend when merging concurrent changes
    pub child_node_ids: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GodotConnection {
    pub signal: String,
    pub from_node_id: i32,
    pub to_node_id: i32,
    pub method: String,
    pub flags: Option<i64>,
	pub from_uid_path: Option<Vec<i32>>,
	pub to_uid_path: Option<Vec<i32>>,
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
    pub idx: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubResourceNode {
    pub id: String,
    pub resource_type: String,
    pub properties: HashMap<String, OrderedProperty>, // key value pairs below the section header
    pub idx: i64,
}
// test
// test 3
//test 2

impl GodotScene {
    pub fn get_node_path(&self, node_id: i32) -> String {
        let mut path = String::new();
        let mut current_id = node_id;
        let root_node_id = self.root_node_id.unwrap_or(UNIQUE_SCENE_ID_UNASSIGNED);

        if node_id == root_node_id {
            return ".".to_string();
        }

        loop {
            let node = self.nodes.get(&current_id).unwrap();

            path = if path.is_empty() {
                node.name.clone()
            } else {
                format!("{}/{}", node.name, path)
            };

            match node.parent_id {
                Some(parent_id) => {
                    current_id = parent_id;

                    if current_id == root_node_id {
                        return path;
                    }
                }
                None => {
                    return path;
                }
            }
        }
    }

    fn get_ext_resource_path(&self, ext_resource_id: &str) -> Option<String> {
        let ext_resource = self.ext_resources.get(ext_resource_id);
        if let Some(ext_resource) = ext_resource {
            return Some(ext_resource.path.clone());
        }
        None
    }

	pub fn reconcile_subresource_node(&self, tx: &mut Transaction, resource_obj: automerge::ObjId, resource: &SubResourceNode) {
		tx.put(
			&resource_obj,
			"resource_type",
			resource.resource_type.clone(),
		)
		.unwrap();

		tx.put(&resource_obj, "id", resource.id.clone()).unwrap();
		tx.put(&resource_obj, "idx", resource.idx).unwrap();

		let properties_obj = tx
			.get_obj_id(&resource_obj, "properties")
			.unwrap_or_else(|| {
				tx.put_object(&resource_obj, "properties", ObjType::Map)
					.unwrap()
			});

		let mut properties_to_delete = tx.keys(&properties_obj).collect::<HashSet<_>>();

		// Add or update properties
		for (key, property) in &resource.properties {
			let value_obj = tx
				.get_obj_id(&properties_obj, key)
				.unwrap_or_else(|| tx.put_object(&properties_obj, key, ObjType::Map).unwrap());

			let value = tx.get_string(&value_obj, "value");
			if value != Some(property.value.clone()) {
				let _ = tx.put(&value_obj, "value", property.value.clone());
			}

			let order = tx.get_int(&value_obj, "order");
			if order != Some(property.order) {
				let _ = tx.put(&value_obj, "order", property.order);
			}
			properties_to_delete.remove(key);
		}

		// Remove properties that no longer exist
		for key in properties_to_delete {
			tx.delete(&properties_obj, &key).unwrap();
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
            .get_obj_id(&structured_content, "connections")
            .unwrap_or_else(|| {
                tx.put_object(&structured_content, "connections", ObjType::Map)
                    .unwrap()
            });

        // Store Scene Metadata
        tx.put(&structured_content, "uid", self.uid.clone())
            .unwrap();
        tx.put(&structured_content, "load_steps", self.load_steps)
            .unwrap();
        tx.put(&structured_content, "format", self.format).unwrap();
        if let Some(script_class) = &self.script_class {
            tx.put(&structured_content, "script_class", script_class.clone())
                .unwrap();
        }
        tx.put(
            &structured_content,
            "resource_type",
            self.resource_type.clone(),
        )
        .unwrap();

        // Store main resource if it exists
        if let Some(main_resource) = &self.main_resource {
            let main_resource_obj = tx
                .get_obj_id(&structured_content, "main_resource")
                .unwrap_or_else(|| {
                    tx.put_object(&structured_content, "main_resource", ObjType::Map)
                        .unwrap()
                });
			self.reconcile_subresource_node(tx, main_resource_obj, main_resource);
        } else if tx
            .get_obj_id(&structured_content, "main_resource")
            .is_some()
        {
            // Remove main_resource if it exists but we don't have one
            tx.delete(&structured_content, "main_resource").unwrap();
        } else if self.resource_type != "PackedScene" {
            tracing::error!("PackedScene with no main resource!!");
        }

        // Store root node id
        if let Some(root_node_id) = &self.root_node_id {
            tx.put(&structured_content, "root_node_id", root_node_id.clone())
                .unwrap();
        } else if tx.get_string(&structured_content, "root_node_id").is_some() {
            // Remove root node id if it exists but we don't have one
            tx.delete(&structured_content, "root_node_id").unwrap();
        }

        // Reconcile external resources
        let ext_resources = tx
            .get_obj_id(&structured_content, "ext_resources")
            .unwrap_or_else(|| {
                tx.put_object(&structured_content, "ext_resources", ObjType::Map)
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
            tx.put(&resource_obj, "idx", resource.idx).unwrap();
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
            .get_obj_id(&structured_content, "sub_resources")
            .unwrap_or_else(|| {
                tx.put_object(&structured_content, "sub_resources", ObjType::Map)
                    .unwrap()
            });

        for (id, resource) in &self.sub_resources {
            let resource_obj = tx
                .get_obj_id(&sub_resources, id)
                .unwrap_or_else(|| tx.put_object(&sub_resources, id, ObjType::Map).unwrap());
			self.reconcile_subresource_node(tx, resource_obj, resource);
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
                tx.put(&node_obj, "parent", parent_id.clone()).unwrap();
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

            if let Some(node_paths) = &node.node_paths {
                tx.put(&node_obj, "node_paths", node_paths.clone()).unwrap();
            } else if tx.get_string(&node_obj, "node_paths").is_some() {
                tx.delete(&node_obj, "node_paths").unwrap();
            }

            // Store properties
            let properties_obj = tx.get_obj_id(&node_obj, "properties").unwrap_or_else(|| {
                tx.put_object(&node_obj, "properties", ObjType::Map)
                    .unwrap()
            });

            // Get existing properties to check for deletions
            let mut properties_to_delete = tx.keys(&properties_obj).collect::<HashSet<_>>();

            // Add or update properties
            for (key, property) in &node.properties {
                let value_obj = tx
                    .get_obj_id(&properties_obj, key)
                    .unwrap_or_else(|| tx.put_object(&properties_obj, key, ObjType::Map).unwrap());

                // tracing::debug!("reconcile {:?} {:?}", key, property);

                let value = tx.get_string(&value_obj, "value");
                if value != Some(property.value.clone()) {
                    let _ = tx.put(&value_obj, "value", property.value.clone());
                }

                let order = tx.get_int(&value_obj, "order");
                if order != Some(property.order) {
                    let _ = tx.put(&value_obj, "order", property.order);
                }
                properties_to_delete.remove(key);
            }

            // Remove properties that no longer exist
            for key in properties_to_delete {
                tx.delete(&properties_obj, &key).unwrap();
            }

            // Store child node IDs
            let children_obj: automerge::ObjId = tx
                .get_obj_id(&node_obj, "child_node_ids")
                .unwrap_or_else(|| {
                    tx.put_object(&node_obj, "child_node_ids", ObjType::Map)
                        .unwrap()
                });

            // reconcile child node ids

            let mut child_node_ids_to_remove = tx.keys(&children_obj).collect::<HashSet<_>>();

            let mut current_child_number = 0;

            for child_node_id in node.child_node_ids.iter() {
                child_node_ids_to_remove.remove(child_node_id);

                match tx.get_int(&children_obj, child_node_id) {
                    // child exists, check if we need to change the number to reflect the new order
                    Some(number) => {
                        if number != current_child_number {
                            let _ = tx.put(&children_obj, child_node_id, current_child_number);
                        }
                    }
                    // child does not exist, add it to the map with the next child number
                    None => {
                        tx.put(&children_obj, child_node_id, current_child_number)
                            .unwrap();
                    }
                };
				current_child_number += 1;
            }

            // remove child node ids that are not in the new node
            for child_node_id in child_node_ids_to_remove {
                tx.delete(&children_obj, &child_node_id).unwrap();
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
        // reconcile editable instances
        // editable instances are stored as an array in the doc, similar to child node IDs
        let editable_instances_obj: automerge::ObjId = tx
            .get_obj_id(&structured_content, "editable_instances")
            .unwrap_or_else(|| {
                tx.put_object(&structured_content, "editable_instances", ObjType::List)
                    .unwrap()
            });

        for (i, path) in self.editable_instances.iter().enumerate() {
            if let Some(current_path) = tx.get_string(&editable_instances_obj, i) {
                if current_path != *path {
                    tx.put(&editable_instances_obj, i, path.clone()).unwrap();
                }
            } else {
                let _ = tx.insert(&editable_instances_obj, i, path.clone());
            }
        }
    }

    pub fn hydrate(doc: &Automerge, path: &str) -> Result<Self, String> {
        Self::hydrate_at(&doc, path, &doc.get_heads())
    }


	fn hydrate_subresource_node(doc: &Automerge, sub_resource_obj: automerge::ObjId, sub_resource_id: String, heads: &Vec<ChangeHash>) -> Result<SubResourceNode, String> {

            let resource_type = doc
                .get_string_at(&sub_resource_obj, "resource_type", &heads)
                .ok_or_else(|| {
                    format!("Could not find resource_type for ID: {}", sub_resource_id)
                })?;

            let id = doc
                .get_string_at(&sub_resource_obj, "id", &heads)
                .ok_or_else(|| format!("Could not find id for ID: {}", sub_resource_id))?;

            let idx = doc
                .get_int_at(&sub_resource_obj, "idx", &heads)
                .ok_or_else(|| format!("Could not find idx for ID: {}", sub_resource_id))?;

            let properties_obj = doc
                .get_obj_id_at(&sub_resource_obj, "properties", &heads)
                .ok_or_else(|| {
                    format!(
                        "Could not find properties object for ID: {}",
                        sub_resource_id
                    )
                })?;

            let mut properties = HashMap::new();
            for key in doc.keys_at(&properties_obj, &heads) {
                let property_obj = doc.get_obj_id_at(&properties_obj, &key, &heads).ok_or_else(|| {
                    format!("Could not find property object for key: {}", key)
                })?;
                let order = doc.get_int_at(&property_obj, "order", &heads).ok_or_else(|| {
                    format!("Could not find order for key: {}", key)
                })?;
                let value = doc.get_string_at(&property_obj, "value", &heads).ok_or_else(|| {
                    format!("Could not find value for key: {}", key)
                })?;

                properties.insert(key, OrderedProperty { value, order });
            }

            let sub_resource = SubResourceNode {
                id,
                resource_type,
                properties,
                idx,
            };

			Ok(sub_resource)
	}

    pub fn hydrate_at(
        doc: &Automerge,
        path: &str,
        heads: &Vec<ChangeHash>,
    ) -> Result<Self, String> {
        // Get the files object
        let files = doc
            .get_obj_id_at(ROOT, "files", &heads)
            .ok_or_else(|| "Could not find files object in document".to_string())?;

        // Get the specific file at the given path
        let scene_file = doc
            .get_obj_id_at(&files, path, &heads)
            .ok_or_else(|| format!("Could not find file at path: {}", path))?;

        // Get the structured content
        let structured_content = doc
            .get_obj_id_at(&scene_file, "structured_content", &heads)
            .ok_or_else(|| "Could not find structured_content in file".to_string())?;

        // Get the uid
        let uid = doc
            .get_string_at(&structured_content, "uid", &heads)
            .ok_or_else(|| "Could not find uid in scene_file".to_string())?;

        let load_steps = doc
            .get_int_at(&structured_content, "load_steps", &heads)
            .ok_or_else(|| "Could not find load_steps in scene_file".to_string())?;

        let format = doc
            .get_int_at(&structured_content, "format", &heads)
            .ok_or_else(|| "Could not find format in scene_file".to_string())?;

        let script_class = doc.get_string_at(&structured_content, "script_class", &heads);
        let resource_type = doc
            .get_string_at(&structured_content, "resource_type", &heads)
            .unwrap_or("PackedScene".to_string());

        // Get main resource if it exists
        let main_resource = if let Some(main_resource_obj) =
            doc.get_obj_id_at(&structured_content, "main_resource", &heads)
        {
			let result = Self::hydrate_subresource_node(doc, main_resource_obj, "main_resource".to_string(), heads);
            if let Ok(sub_resource) = result {
                Some(sub_resource)
            } else if let Err(e) = result {
				tracing::error!("Error hydrating main resource: {}", e);
                None
            } else {
				tracing::error!("Error hydrating main resource: unknown error");
                None
            }
        } else {
			if resource_type != "PackedScene" {
				tracing::error!("resource with no main resource!!");
			}
            None
        };

        // Get the nodes object
        let nodes_id = doc
            .get_obj_id_at(&structured_content, "nodes", &heads)
            .ok_or_else(|| "Could not find nodes in structured_content".to_string())?;

        let root_node_id = if let Some(root_node_id) = doc.get_int_at(&structured_content, "root_node_id", &heads) {
            Some(root_node_id as i32)
        } else {
            None
        };

        // Create a map to store the nodes

        // Iterate through all external resources

        let ext_resources_id = doc
            .get_obj_id_at(&structured_content, "ext_resources", &heads)
            .ok_or_else(|| "Could not find ext_resources in scene_file".to_string())?;

        let mut sorted_ext_resources = Vec::new();
        for resource_id in doc.keys_at(&ext_resources_id, &heads) {
            let resource_obj = doc
                .get_obj_id_at(&ext_resources_id, &resource_id, &heads)
                .ok_or_else(|| format!("Could not find resource object for ID: {}", resource_id))?;

            let resource_type = doc
                .get_string_at(&resource_obj, "resource_type", &heads)
                .ok_or_else(|| format!("Could not find resource_type for ID: {}", resource_id))?;

            let path = doc
                .get_string_at(&resource_obj, "path", &heads)
                .ok_or_else(|| format!("Could not find path for ID: {}", resource_id))?;

            let id = doc
                .get_string_at(&resource_obj, "id", &heads)
                .ok_or_else(|| format!("Could not find id for ID: {}", resource_id))?;

            let idx = doc
                .get_int_at(&resource_obj, "idx", &heads)
                .ok_or_else(|| format!("Could not find idx for ID: {}", resource_id))?;

            let uid = doc.get_string_at(&resource_obj, "uid", &heads);

            let external_resource = ExternalResourceNode {
                resource_type,
                uid,
                path,
                id,
                idx,
            };
            sorted_ext_resources.push((resource_id.clone(), external_resource));
        }
        sorted_ext_resources.sort_by_key(|(_, resource)| resource.idx);
        let ext_resources = sorted_ext_resources
            .into_iter()
            .map(|(id, resource)| (id.clone(), resource))
            .collect();

        // Itereate through all sub resources

        let sub_resources_id = doc
            .get_obj_id_at(&structured_content, "sub_resources", &heads)
            .ok_or_else(|| "Could not find sub_resources in scene_file".to_string())?;

        let mut sorted_sub_resources = Vec::new();
        for sub_resource_id in doc.keys_at(&sub_resources_id, &heads) {
            let sub_resource_obj = doc
                .get_obj_id_at(&sub_resources_id, &sub_resource_id, &heads)
                .ok_or_else(|| {
                    format!(
                        "Could not find sub_resource object for ID: {}",
                        sub_resource_id
                    )
                })?;

            let sub_resource = Self::hydrate_subresource_node(doc, sub_resource_obj, sub_resource_id.clone(), heads)?;
            sorted_sub_resources.push((sub_resource_id.clone(), sub_resource));
        }
        sorted_sub_resources.sort_by_key(|(_, resource)| resource.idx);
        let sub_resources = sorted_sub_resources
            .into_iter()
            .map(|(id, resource)| (id.clone(), resource))
            .collect();

        // Iterate through all node IDs in the nodes object

        let mut nodes = HashMap::new();

        for node_id in doc.keys_at(&nodes_id, &heads) {
            // Get the node object
            let node_obj = doc
                .get_obj_id_at(&nodes_id, &node_id, &heads)
                .ok_or_else(|| format!("Could not find node object for ID: {}", node_id))?;

            // Extract node properties
            let id: i32 = doc
                .get_int_at(&node_obj, "id", &heads)
                .unwrap_or(0) as i32;
            let name = doc
                .get_string_at(&node_obj, "name", &heads)
                .ok_or_else(|| format!("Node {} is missing required name property", node_id))?;

            // Determine if this is a type or instance
            let type_or_instance =
                if let Some(type_name) = doc.get_string_at(&node_obj, "type", &heads) {
                    TypeOrInstance::Type(type_name)
                } else if let Some(instance_id) = doc.get_string_at(&node_obj, "instance", &heads) {
                    TypeOrInstance::Instance(instance_id)
                } else {
                    return Err(format!(
                        "Node {} is missing both type and instance properties",
                        node_id
                    ));
                };

            // Get optional properties
            let parent_id = doc.get_string_at(&node_obj, "parent", &heads);
			let parent_id_path = doc.get_int(&node_obj, "parent_id_path", &heads);
            let owner = doc.get_string_at(&node_obj, "owner", &heads);
			let owner_uid_path = doc.get_int32_array_at(&node_obj, "owner_uid_path", &heads);
            let index = doc.get_int_at(&node_obj, "index", &heads).map(|i| i);
            let groups = doc.get_string_at(&node_obj, "groups", &heads);
            let node_paths = doc.get_string_at(&node_obj, "node_paths", &heads);
            // Get node properties
            let properties_obj = doc
                .get_obj_id_at(&node_obj, "properties", &heads)
                .ok_or_else(|| format!("Could not find properties object for node: {}", node_id))?;
            let mut properties = HashMap::new();
            for key in doc.keys_at(&properties_obj, &heads) {
                let property_obj = doc.get_obj_id_at(&properties_obj, &key, &heads).unwrap();

                let value = doc
                    .get_string_at(&property_obj, "value", &heads)
                    .ok_or_else(|| format!("Could not find value for property: {}", key))?;

                let order = doc.get_int_at(&property_obj, "order", &heads).unwrap();

                properties.insert(key, OrderedProperty { value, order });
            }

            // Get child node IDs
            let children_obj = doc
                .get_obj_id_at(&node_obj, "child_node_ids", &heads)
                .unwrap();

            let mut child_node_ids = doc.keys_at(&children_obj, &heads).collect::<Vec<_>>();

            child_node_ids.sort_by(|a, b| {
                let a_idx = doc.get_int_at(&children_obj, a, &heads).unwrap();
                let b_idx = doc.get_int_at(&children_obj, b, &heads).unwrap();
                a_idx.cmp(&b_idx)
            });

            // Create the node
            let node = GodotNode {
                id,
                name,
                type_or_instance,
                parent_id,
				parent_id_path,
                owner,
				owner_uid_path,
                index,
                groups,
                properties,
                child_node_ids,
                node_paths,
            };

            // Add the node to our map
            nodes.insert(node_id, node);
        }

        // Iterate through all connections
        let mut connections = HashMap::new();

        let connections_id = doc
            .get_obj_id_at(&structured_content, "connections", &heads)
            .ok_or_else(|| "Could not find connections in scene document".to_string())?;

        for connection_id in doc.keys_at(&connections_id, &heads) {
            let connection_obj = doc
                .get_obj_id_at(&connections_id, &connection_id, &heads)
                .ok_or_else(|| {
                    format!("Could not find connection object for ID: {}", connection_id)
                })?;

            let signal = doc
                .get_string_at(&connection_obj, "signal", &heads)
                .ok_or_else(|| {
                    format!("Could not find signal for connection: {}", connection_id)
                })?;

            let from_node_id = doc
                .get_string_at(&connection_obj, "from_node_id", &heads)
                .ok_or_else(|| {
                    format!(
                        "Could not find from_node_id for connection: {}",
                        connection_id
                    )
                })?;

            let to_node_id = doc
                .get_string_at(&connection_obj, "to_node_id", &heads)
                .ok_or_else(|| {
                    format!(
                        "Could not find to_node_id for connection: {}",
                        connection_id
                    )
                })?;

            let method = doc
                .get_string_at(&connection_obj, "method", &heads)
                .ok_or_else(|| {
                    format!("Could not find method for connection: {}", connection_id)
                })?;

            let flags = doc.get_int_at(&connection_obj, "flags", &heads);

            let unbinds = doc.get_int_at(&connection_obj, "unbinds", &heads);

            let binds = doc.get_string_at(&connection_obj, "binds", &heads);

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

        let mut editable_instances = HashSet::new();
        let editable_instances_obj =
            doc.get_obj_id_at(&structured_content, "editable_instances", &heads);
        if let Some(editable_instances_obj) = editable_instances_obj {
            let length = doc.length_at(&editable_instances_obj, &heads);
            for i in 0..length {
                if let Some(path) = doc.get_string_at(&editable_instances_obj, i, &heads) {
                    editable_instances.insert(path);
                }
            }
        }

        // Create a GodotScene with default values for everything except nodes
        Ok(GodotScene {
            load_steps,
            format,
            uid,
            script_class,
            resource_type,
            root_node_id,
            ext_resources,
            sub_resources,
            nodes,
            connections,
            editable_instances,
            main_resource,
			requires_resave: false
        })
    }

    pub fn serialize(&self) -> String {
        let mut output = String::new();

        if (self.resource_type != "PackedScene") {
            output.push_str(&format!("[gd_resource type=\"{}\"", self.resource_type));
            if let Some(script_class) = &self.script_class {
                output.push_str(&format!(" script_class=\"{}\"", script_class));
            }
        } else {
            output.push_str("[gd_scene");
        }
        if self.load_steps != 0 {
            output.push_str(&format!(" load_steps={}", self.load_steps));
        }
        output.push_str(&format!(
            " format={} uid=\"{}\"]\n\n",
            self.format, self.uid
        ));

        // External resources

        // sort resources by idx ascending
        let mut sorted_ext_resources: Vec<(&String, &ExternalResourceNode)> =
            self.ext_resources.iter().collect();
        sorted_ext_resources.sort_by_key(|(_, resource)| resource.idx);

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

        // Sub-resources sorted by idx ascending
        let mut sorted_sub_resources: Vec<(&String, &SubResourceNode)> =
            self.sub_resources.iter().collect();
        sorted_sub_resources.sort_by_key(|(_, resource)| resource.idx);
        for (_, resource) in sorted_sub_resources {
            output.push_str(&format!(
                "[sub_resource type=\"{}\" id=\"{}\"]\n",
                resource.resource_type, resource.id
            ));

            // Properties sorted by order number of each property
            let mut sorted_props: Vec<(&String, &OrderedProperty)> =
                resource.properties.iter().collect();
            sorted_props.sort_by(|(_, a), (_, b)| a.order.cmp(&b.order));
            for (key, property) in sorted_props {
                output.push_str(&format!("{} = {}\n", key, property.value));
            }

            output.push('\n');
        }

        // Main resource if it exists
        if let Some(main_resource) = &self.main_resource {
            output.push_str(&format!("[resource]\n"));

            // Properties sorted by order number of each property
            let mut sorted_props: Vec<(&String, &OrderedProperty)> =
                main_resource.properties.iter().collect();
            sorted_props.sort_by(|(_, a), (_, b)| a.order.cmp(&b.order));
            for (key, property) in sorted_props {
                output.push_str(&format!("{} = {}\n", key, property.value));
            }
            // short circuit if we have a main resource, no nodes or connections
            return output;
        } else if self.resource_type != "PackedScene" {
            tracing::error!("resource with no resource tag!!");
        }

		let mut node_paths_visited: HashMap<i32, i64> = HashMap::new();
        if !self.nodes.is_empty() && self.root_node_id.is_some() {
            if let Some(root_node) = self.nodes.get(&self.root_node_id.unwrap()) {
                self.serialize_node(&mut output, root_node, &mut node_paths_visited);
				if self.connections.len() == 0  && self.editable_instances.len() == 0 {
					// prevent an extra trailing new line
					output.pop();
				}
            }
        }

        let mut connections: Vec<(&String, &GodotConnection)> =
            self.connections.iter().collect::<Vec<_>>();

        connections.sort_by(|(id_a, conn_a), (id_b, conn_b)| {
			let sort_a = node_paths_visited.get(&conn_a.from_node_id).unwrap_or(&-1);
			let sort_b = node_paths_visited.get(&conn_b.from_node_id).unwrap_or(&-1);
			if sort_a == sort_b {
				// compare the signal
				conn_a.signal.cmp(&conn_b.signal)
			} else {
				sort_a.cmp(sort_b)
			}
		});

        for (_, connection) in connections {
            let from_path = self.get_node_path(connection.from_node_id);
            let to_path = self.get_node_path(connection.to_node_id);

            output.push_str(&format!(
                "[connection signal=\"{}\" from=\"{}\" to=\"{}\" method=\"{}\"",
                connection.signal, from_path, to_path, connection.method
            ));
            if let Some(flags) = connection.flags {
                output.push_str(&format!(" flags={}", flags));
            }
			if let Some(from_uid_path) = &connection.from_uid_path {
				output.push_str(&format!(" from_uid_path={}", serialize_int32_array(from_uid_path)));
			}
			if let Some(to_uid_path) = &connection.to_uid_path {
				output.push_str(&format!(" to_uid_path={}", serialize_int32_array(to_uid_path)));
			}
            if let Some(unbinds) = connection.unbinds {
                output.push_str(&format!(" unbinds={}", unbinds));
            }
            if let Some(binds) = &connection.binds {
                output.push_str(&format!(" binds={}", binds));
            }
            output.push_str("]\n");
        }

        for path in self.editable_instances.iter() {
            output.push_str(&format!("[editable path=\"{}\"]\n", path));
        }
        output
    }

    fn serialize_node(&self, output: &mut String, node: &GodotNode, node_paths_visited: &mut HashMap<i32, i64>) {
		// name, type, parent, parent_id_path, owner, owner_uid_path, index, unique_id, node_paths, groups, instance_placeholder, instance
        output.push_str(&format!("[node name=\"{}\"", node.name));

        if let TypeOrInstance::Type(t) = &node.type_or_instance {
            output.push_str(&format!(" type=\"{}\"", t));
        }

        if let Some(parent_id) = node.parent_id {
            let parent_name = if self.root_node_id.is_some() && parent_id == self.root_node_id.unwrap() {
                ".".to_string()
            } else {
                self.get_node_path(parent_id)
            };

            output.push_str(&format!(" parent=\"{}\"", parent_name));
			if let Some(parent_id_path) = &node.parent_id_path {
				output.push_str(&format!(" parent_id_path={}", serialize_int32_array(parent_id_path)));
			}
        }

        if let Some(owner) = &node.owner {
            output.push_str(&format!(" owner=\"{}\"", owner));
			if let Some(owner_uid_path) = &node.owner_uid_path {
				output.push_str(&format!(" owner_uid_path={}", serialize_int32_array(owner_uid_path)));
			}
        }

        if let Some(index) = &node.index {
            output.push_str(&format!(" index={}", index));
        }

		output.push_str(&format!(" unique_id={}", node.id));

        if let Some(node_paths) = &node.node_paths {
            output.push_str(&format!(" node_paths={}", node_paths));
        }

        if let Some(groups) = &node.groups {
            output.push_str(&format!(" groups={}", groups));
        }

		if let Some(instance_placeholder) = &node.instance_placeholder {
			output.push_str(&format!(" instance_placeholder={}", instance_placeholder));
		}

        if let TypeOrInstance::Instance(i) = &node.type_or_instance {
            output.push_str(&format!(" instance={}", i));
        }

		node_paths_visited.insert(node.id, node_paths_visited.len() as i64);




        output.push_str("]\n");

        // Properties sorted by order number of each property
        let mut sorted_props: Vec<(&String, &OrderedProperty)> = node.properties.iter().collect();
        sorted_props
            .sort_by(|(_, property_a), (_, property_b)| property_a.order.cmp(&property_b.order));
        for (key, property) in sorted_props {
            output.push_str(&format!("{} = {}\n", key, property.value));
        }

        // Always add a blank line after a node's properties
        output.push('\n');

        // Recursively serialize children
        for child_id in &node.child_node_ids {
            if let Some(child_node) = self.nodes.get(child_id) {
                self.serialize_node(output, child_node, node_paths_visited);
            }
        }
    }

	pub fn get_node(&self, node_id: i32) -> Option<&GodotNode> {
		self.nodes.get(&node_id)
	}

}

#[inline]
fn parse_int32_array(string: &String) -> Vec<i32> {
	string.strip_prefix("PackedInt32Array(").unwrap().strip_suffix(")").unwrap().trim().split(',').map(|s| s.trim().parse::<i32>().unwrap_or(0)).collect()
}

#[inline]
fn serialize_int32_array(array: &Vec<i32>) -> String {
	format!("PackedInt32Array({})", array.iter().map(|i| i.to_string()).collect::<Vec<String>>().join(", "))
}

#[derive(Debug, Clone)]
pub struct SceneMetadata {
    pub load_steps: i64,
    pub format: i64,
    pub uid: String,
    pub script_class: Option<String>,
    pub resource_type: String,
}

pub fn recognize_scene(source: &String) -> bool {
    // go line by line until we find a line that does not start with a comment (i.e. ;) and is not empty
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(";") && !trimmed.is_empty() {
            // check if the line starts with "[gd_resource" or "[gd_scene"
            if trimmed.starts_with("["){
                let line_after_bracket = &trimmed[1..].trim();
                if line_after_bracket.starts_with("gd_resource") || line_after_bracket.starts_with("gd_scene") {
				// if line_after_bracket.starts_with("gd_scene") {
                    return true;
                }
            }
            // gd_resource and gd_scene have to be the first non-comment, non-empty line; if we find another line that is not a comment or empty, we can return false
            break;
        }
    }
    false
}
pub fn parse_scene(source: &String) -> Result<GodotScene, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_godot_resource::LANGUAGE.into())
        .expect("Error loading godot resource grammar");

    let result = parser.parse(source, None);

    let mut parsed_node_ids = HashSet::new();

	let mut required_resave = false;
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
                Query::new(&tree_sitter_godot_resource::LANGUAGE.into(), query).expect("Invalid query");
            let mut query_cursor = QueryCursor::new();
            let mut matches = query_cursor.matches(&query, tree.root_node(), content_bytes);

            // Initialize with default values
            let mut scene_metadata: Option<SceneMetadata> = None;
            let mut nodes: HashMap<i32, GodotNode> = HashMap::new();
			let mut node_arr: Vec<(GodotNode, Option<String>)> = Vec::new();
            let mut ext_resources: HashMap<String, ExternalResourceNode> = HashMap::new();
            let mut sub_resources: HashMap<String, SubResourceNode> = HashMap::new();
            let mut connections: HashMap<String, GodotConnection> = HashMap::new();
			let mut connections_arr: Vec<(String, String, GodotConnection)> = Vec::new();
            let mut root_node_id: Option<i32> = None;
            let mut main_resource: Option<SubResourceNode> = None;
            let mut editable_instances: HashSet<String> = HashSet::new();
            // Create an index to map node paths to node ids
            let mut node_id_by_node_path: HashMap<String, i32> = HashMap::new();
            let mut ext_resource_idx = 0;
            let mut sub_resource_idx = 0;
            while let Some(m) = matches.next() {
                let mut heading = HashMap::new();
                let mut properties = Vec::new();
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
                                        let key = text.to_string();
										properties.push((
											key,
											OrderedProperty {
												value: value.to_string(),
												order: properties.len() as i64,
											},
										));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                let mut resource_type: String = "PackedScene".to_string();
                // GD_RESOURCE HEADER
                if section_id == "gd_resource" {
                    let load_steps = heading
                        .get("load_steps")
                        .and_then(|ls| ls.parse::<i64>().ok())
                        .unwrap_or(0);

                    let format = match heading.get("format").and_then(|f| f.parse::<i64>().ok()) {
                        Some(format) => format,
                        None => {
                            return Err("Missing required 'format' attribute in gd_resource header"
                                .to_string())
                        }
                    };

                    let script_class: Option<String> = match heading.get("script_class") {
                        Some(script_class) => Some(unquote(&script_class)),
                        None => None,
                    };

                    let uid: String = match heading.get("uid") {
                        Some(uid) => unquote(&uid),
                        None => {
                            return Err("Missing required 'uid' attribute in gd_resource header"
                                .to_string())
                        }
                    };

                    resource_type = match heading.get("type").cloned() {
                        Some(resource_type) => unquote(&resource_type),
                        None => {
                            return Err("Missing required 'type' attribute in gd_resource header"
                                .to_string())
                        }
                    };

                    scene_metadata = Some(SceneMetadata {
                        load_steps,
                        format,
                        uid,
                        script_class,
                        resource_type,
                    });

                // RESOURCE
                } else if section_id == "resource" {
                    main_resource = Some(SubResourceNode {
                        id: "".to_string(), // Resource sections don't have IDs
                        resource_type,
                        properties: properties.into_iter().collect(),
                        idx: 0,
                    });

                // GD_SCENE HEADER
                } else if section_id == "gd_scene" {
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
                        Some(uid) => unquote(&uid),
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
                        resource_type: "PackedScene".to_string(),
                        script_class: None,
                    });

                // NODE
                } else if section_id == "node" {
                    // Check if node has a patchwork_id in metadata
                    let mut node_id_num = match heading.get("unique_id") {
                        Some(unique_id) => unique_id.parse::<i32>().unwrap_or(UNIQUE_SCENE_ID_UNASSIGNED),
                        None => UNIQUE_SCENE_ID_UNASSIGNED
                    };
					if node_id_num == UNIQUE_SCENE_ID_UNASSIGNED {
						required_resave = true;
					} else {
						parsed_node_ids.insert(node_id_num);
					}


                    let name = match heading.get("name") {
                        Some(name) => unquote(name),
                        None => {
                            return Err(
                                "Missing required 'name' attribute in node section".to_string()
                            )
                        }
                    };

					let instance_placeholder = heading.get("instance_placeholder").cloned().map(|p| unquote(&p));

					let parent_id_path = heading.get("parent_id_path").cloned().map(|p| parse_int32_array(&p));

					let owner_uid_path = heading.get("owner_uid_path").cloned().map(|p| parse_int32_array(&p));

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

                    let node_paths = heading.get("node_paths").cloned().map(|p| unquote(&p));
                    let parent_path = heading.get("parent").cloned().map(|p| unquote(&p));
					let parent_id = None;

                    let node = GodotNode {
                        id: node_id_num,
                        name,
                        type_or_instance,
                        instance_placeholder,
                        parent_id,
						parent_id_path,
                        owner: heading.get("owner").cloned().map(|o| unquote(&o)),
						owner_uid_path,
                        index: heading.get("index").and_then(|i| i.parse::<i64>().ok()),
                        groups: heading.get("groups").cloned(),
                        properties: properties.into_iter().collect(),
                        child_node_ids: Vec::new(),
                        node_paths,
                    };
					node_arr.push((node, parent_path));

                    // nodes.insert(node_id_num.to_string(), node);

                // EXTERNAL RESOURCE
                //
                } else if section_id == "ext_resource" {
                    // Add to ext_resources map

                    let ext_resource_type = match heading.get("type").cloned() {
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
                            resource_type: ext_resource_type,
                            uid,
                            path,
                            id,
                            idx: ext_resource_idx,
                        },
                    );

                    ext_resource_idx += 1;
                // SUB-RESOURCE
                } else if section_id == "sub_resource" {
                    let id = match heading.get("id").cloned() {
                        Some(id) => unquote(&id),
                        None => {
                            return Err("Missing required 'id' attribute in sub_resource section"
                                .to_string())
                        }
                    };

                    let subresource_type = match heading.get("type").cloned() {
                        Some(resource_type) => unquote(&resource_type),
                        None => {
                            return Err("Missing required 'type' attribute in sub_resource section"
                                .to_string())
                        }
                    };

                    let sub_resource = SubResourceNode {
                        id: id.clone(),
                        resource_type: subresource_type,
                        properties: properties.into_iter().collect(),
                        idx: sub_resource_idx,
                    };

                    sub_resources.insert(id, sub_resource);

                    sub_resource_idx += 1;
                // CONNECTION
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

                    let to_path = match heading.get("to").cloned() {
                        Some(to) => unquote(&to),
                        None => {
                            return Err(
                                "Missing required 'to' attribute in connection section".to_string()
                            )
                        }
                    };

                    let method = match heading.get("method").cloned() {
                        Some(method) => unquote(&method),
                        None => {
                            return Err("Missing required 'method' attribute in connection section"
                                .to_string())
                        }
                    };

                    let flags = heading.get("flags").and_then(|f| f.parse::<i64>().ok());

					let from_uid_path = heading.get("from_uid_path").cloned().map(|p| parse_int32_array(&p));

					let to_uid_path = heading.get("to_uid_path").cloned().map(|p| parse_int32_array(&p));

                    let unbinds = heading.get("unbinds").and_then(|u| u.parse::<i64>().ok());

                    let binds = heading.get("binds").cloned().map(|b| unquote(&b));

                    let connection = GodotConnection {
                        signal,
						from_node_id: UNIQUE_SCENE_ID_UNASSIGNED,
						to_node_id: UNIQUE_SCENE_ID_UNASSIGNED,
                        method,
                        flags,
						from_uid_path,
						to_uid_path,
                        unbinds,
                        binds,
                    };

                    // connections.insert(connection.id().clone(), connection);
					connections_arr.push((from_path, to_path, connection));
                } else if section_id == "editable" {
                    // just has a path attribute
                    let path = match heading.get("path").cloned() {
                        Some(path) => unquote(&path),
                        None => {
                            return Err(
                                "Missing required 'path' attribute in editable section".to_string()
                            )
                        }
                    };
                    editable_instances.insert(path);
                }
            }

			for (mut node, parent_path) in node_arr {
				if node.id == UNIQUE_SCENE_ID_UNASSIGNED {
					node.id = rand::rng().random_range(0..=i32::MAX);
					while parsed_node_ids.contains(&node.id) {
						node.id = rand::rng().random_range(0..=i32::MAX);
					}
					parsed_node_ids.insert(node.id);
				}
				let name = node.name.clone();
				node.parent_id = match parent_path {
					Some(parent_path) => {
						if parent_path == "." {
							node_id_by_node_path.insert(name.clone(), node.id);
						} else {
							node_id_by_node_path
								.insert(format!("{}/{}", parent_path, name), node.id);
						}

						match node_id_by_node_path.get(&parent_path) {
							Some(parent_id) => {
								nodes
									.get_mut(parent_id)
									.unwrap()
									.child_node_ids
									.push(node.id);

								Some(*parent_id)
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
						root_node_id = Some(node.id);
						node_id_by_node_path.insert(".".to_string(), node.id);
						None
					}
				};

				nodes.insert(node.id, node);
			}
			for (from_path, to_path, mut connection) in connections_arr {
				let from_node_id = match node_id_by_node_path.get(&from_path) {
					Some(node_id) => node_id.clone(),
					None => {
						return Err(format!(
							"Can't find node \"{}\", {:?}",
							from_path, node_id_by_node_path
						))
					}
				};


				let to_node_id = match node_id_by_node_path.get(&to_path) {
					Some(node_id) => node_id.clone(),
					None => return Err(format!("Can't find node \"{}\"", from_path)),
				};

				connection.from_node_id = from_node_id;
				connection.to_node_id = to_node_id;
				connections.insert(connection.id().clone(), connection);
			}

            let scene_metadata = match scene_metadata {
                Some(metadata) => metadata,
                None => return Err(String::from("missing gd_scene header")),
            };

            let root_node_id = match root_node_id {
                Some(id) => Some(id),
                None => None,
            };

            Ok(GodotScene {
                load_steps: scene_metadata.load_steps,
                format: scene_metadata.format,
                uid: scene_metadata.uid,
                script_class: scene_metadata.script_class,
                resource_type: scene_metadata.resource_type,
                root_node_id,
                ext_resources,
                sub_resources,
                nodes,
                connections,
                editable_instances,
                main_resource,
				requires_resave: required_resave,
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

use automerge::{
    Automerge, ChangeHash, ROOT, ReadDoc as AutomergeReadDoc
};
use autosurgeon::{Hydrate, HydrateError, Prop, Reconcile, ReadDoc, Reconciler};
use rand::Rng;
use std::{collections::{HashMap, HashSet}, fmt::Display};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

use crate::helpers::doc_utils::SimpleDocReader;

const UNIQUE_SCENE_ID_UNASSIGNED: i32 = 0;
fn hydrate_nodes<D: ReadDoc>(
    doc: &D,
    obj: &automerge::ObjId,
    prop: Prop<'_>,
) -> Result<HashMap<i32, GodotNode>, HydrateError> {
	let res = HashMap::<String, GodotNode>::hydrate(doc, obj, prop);
	if let Ok(map) = res {
		// convert the map to a HashMap<i32, GodotNode>
		let map = map.into_iter().map(|(k, v)| (k.parse::<i32>().unwrap(), v)).collect();
		Ok(map)
	} else {
		Err(res.err().unwrap())
	}
}

fn reconcile_nodes<R: Reconciler>(outer: &HashMap<i32, GodotNode>, reconciler: R) -> Result<(), R::Error> {
    let string_map: HashMap<String, &GodotNode> = outer.iter().map(|(k, v)| (k.to_string(), v)).collect();
	string_map.reconcile(reconciler)
}

#[derive(Debug, Clone, Hydrate, Reconcile, PartialEq, Eq)]
pub struct GodotScene {
    pub load_steps: i64,
    pub format: i64,
    pub uid: String,
    pub script_class: Option<String>,
    pub resource_type: String,
    pub root_node_id: Option<i32>,
    pub ext_resources: HashMap<String, ExternalResourceNode>,
    pub sub_resources: HashMap<String, SubResourceNode>,
	#[autosurgeon(reconcile = "reconcile_nodes", hydrate = "hydrate_nodes")]
    pub nodes: HashMap<i32, GodotNode>,
    pub connections: HashMap<String, GodotConnection>, // key is concatenation of all properties of the connection
    pub editable_instances: Vec<String>,
    pub main_resource: Option<SubResourceNode>
}

#[derive(Debug, Clone, Hydrate, Reconcile, PartialEq, Eq)]
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

#[derive(Debug, Clone, Hydrate, Reconcile, PartialEq, Eq)]
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

#[derive(Debug, Clone, Hydrate, Reconcile, PartialEq, Eq)]
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

#[derive(Debug, Clone, Hydrate, Reconcile, PartialEq, Eq)]
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
#[derive(Debug, Clone, Hydrate, Reconcile, PartialEq, Eq)]
pub struct ExternalResourceNode {
    pub resource_type: String,
    pub uid: Option<String>,
    pub path: String,
    pub id: String,
    pub idx: i64,
}

#[derive(Debug, Clone, Hydrate, Reconcile, PartialEq, Eq)]
pub struct SubResourceNode {
    pub id: String,
    pub resource_type: String,
    pub properties: HashMap<String, OrderedProperty>, // key value pairs below the section header
    pub idx: i64,
}
// test
// test 3
//test 2

// there is no `hydrate_at` method provided by autosurgeon,
// and there's no way to get transaction_at from an immutable doc,
// so we need to implement a ReadDoc that gets everything at the heads
struct AutomergeDocAtHeads<'c> {
	doc: &'c Automerge,
	heads: &'c Vec<ChangeHash>,
}

impl<'c> ReadDoc for AutomergeDocAtHeads<'c> {
	type Parents<'b> = <Automerge as ReadDoc>::Parents<'b> where Self: 'b;

	fn get_heads(&self) -> Vec<automerge::ChangeHash> {
		self.heads.clone()
	}

	fn get<P: Into<automerge::Prop>>(
		&self,
		obj: &automerge::ObjId,
		prop: P,
	) -> Result<Option<(automerge::Value<'_>, automerge::ObjId)>, automerge::AutomergeError> {
		self.doc.get_at(obj, prop, self.heads)
	}

	fn object_type<O: AsRef<automerge::ObjId>>(&self, obj: O) -> Option<automerge::ObjType> {
		// TODO: this seems to be the way that `Transaction` implements it (with no heads), but need to confirm that this is correct
		automerge::ReadDoc::object_type(self.doc, obj).ok()
	}

	fn map_range<'a, O, R>(&'a self, obj: O, range: R) -> automerge::iter::MapRange<'a, R>
	where
		R: core::ops::RangeBounds<String> + 'a,
		O: AsRef<automerge::ObjId>,
		R: core::ops::RangeBounds<String>,
	{
		self.doc.map_range_at(obj, range, self.heads)
	}

	fn list_range<O: AsRef<automerge::ObjId>, R: core::ops::RangeBounds<usize>>(
		&self,
		obj: O,
		range: R,
	) -> automerge::iter::ListRange<'_, R> {
		self.doc.list_range_at(obj, range, self.heads)
	}

	fn length<O: AsRef<automerge::ObjId>>(&self, obj: O) -> usize {
		self.doc.length_at(obj, self.heads)
	}

	fn text<O: AsRef<automerge::ObjId>>(&self, obj: O) -> Result<String, automerge::AutomergeError> {
		self.doc.text_at(obj, self.heads)
	}

	fn parents<O: AsRef<automerge::ObjId>>(&self, obj: O) -> Result<Self::Parents<'_>, automerge::AutomergeError> {
		self.doc.parents_at(obj, self.heads)
	}
}

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

    // fn get_ext_resource_path(&self, ext_resource_id: &str) -> Option<String> {
    //     let ext_resource = self.ext_resources.get(ext_resource_id);
    //     if let Some(ext_resource) = ext_resource {
    //         return Some(ext_resource.path.clone());
    //     }
    //     None
    // }

	pub fn hydrate_at(
        doc: &Automerge,
        path: &str,
        heads: &Vec<ChangeHash>,
    ) -> Result<Self, String> {
		let doc_at_heads = AutomergeDocAtHeads {
			doc: doc,
			heads: heads,
		};
		let files = doc
		.get_obj_id_at(ROOT, "files", &heads)
		.ok_or_else(|| "Could not find files object in document".to_string())?;

	// Get the specific file at the given path
		let scene_file = doc
		.get_obj_id_at(&files, path, &heads)
		.ok_or_else(|| format!("Could not find file at path: {}", path))?;

		GodotScene::hydrate(&doc_at_heads, &scene_file, "structured_content".into()).map_err(|e| e.to_string())
	}

    pub fn serialize(&self) -> String {
        let mut output = String::new();

        if self.resource_type != "PackedScene" {
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

        connections.sort_by(|(_, conn_a), (_, conn_b)| {
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
            let mut editable_instances: Vec<String> = Vec::new();
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
                    let node_id_num = match heading.get("unique_id") {
                        Some(unique_id) => unique_id.parse::<i32>().unwrap_or(UNIQUE_SCENE_ID_UNASSIGNED),
                        None => return Err("Missing required 'unique_id' attribute in node section".to_string())
                    };
					parsed_node_ids.insert(node_id_num);

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
                    editable_instances.push(path);
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
                main_resource
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

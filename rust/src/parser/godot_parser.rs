use automerge::{
    Automerge, ChangeHash, ROOT, ReadDoc as AutomergeReadDoc
};
use autosurgeon::{Hydrate, HydrateError, Prop, Reconcile, ReadDoc, Reconciler};
use rand::Rng;
use regex::Regex;
use std::{collections::{HashMap, HashSet}, fmt::Display};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

use crate::{helpers::doc_utils::SimpleDocReader, parser::parser_defs::OrderedProperty, project::branch_db::history_ref::{HistoryRef, HistoryRefPath}};

const UNIQUE_SCENE_ID_UNASSIGNED: i32 = 0;
fn hydrate_nodes<D: ReadDoc>(
    doc: &D,
    obj: &automerge::ObjId,
    prop: Prop<'_>,
) -> Result<HashMap<i32, GodotNode>, HydrateError> {
	let res = HashMap::<String, GodotNode>::hydrate(doc, obj, prop);
	if let Ok(map) = res {
		// convert the map to a HashMap<i32, GodotNode>
		let mut map: HashMap<i32, GodotNode> = map.into_iter().map(|(k, v)| (k.parse::<i32>().unwrap(), v)).collect();
        let keys: Vec<i32> = map.keys().cloned().collect();
        // Because Godot stores parents by path and not ID, we gotta dedupe names within children
        for id in keys {
            let parent_id = match map.get(&id).and_then(|n| n.parent_id) {
                Some(pid) => pid,
                None => continue,
            };

            let child_ids = match map.get(&parent_id) {
                Some(parent) => parent.child_node_ids.clone(),
                None => continue,
            };

            // Just increment the name until we're no longer duped.
            // Performant in cases of only 1 duplicate; not very performant for multiple dupes.
            // But still probably OK, since we're only ever performing a single regex per inner loop.
            let mut found = true;
            let re = Regex::new(r"(.*?)(\d+)$").unwrap();
            while found {
                found = false;
                let name = map.get(&id).unwrap().name.clone();
                for child_id in &child_ids {
                    if *child_id == id {
                        continue;
                    }
                    let Some(child) = map.get(child_id) else {
                        continue;
                    };
                    if child.name != name {
                        continue;
                    }

                    // We've got the same parent and the same name.
                    // So increment the number...
                    let node = map.get_mut(&id).unwrap();
                    node.name = if let Some(caps) = re.captures(&node.name) {
                        let prefix = &caps[1];
                        let number: u64 = caps[2].parse().unwrap();
                        format!("{}{}", prefix, number + 1)
                    } else {
                        format!("{}1", node.name)
                    };
                    // ... and try again.
                    found = true;
                    break;
                }
            }

        }

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
pub struct GodotNode {
    pub id: i32,
    pub name: String,
    pub type_or_instance: Option<TypeOrInstance>, // a node may have a type or an instance property
	pub instance_placeholder: Option<String>,
    pub parent_id: Option<i32>,
    pub parent_path_fallback: Option<String>,
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

	fn map_range<'a, O, R>(&'a self, obj: O, range: R) -> automerge::iter::MapRange<'a>
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
	) -> automerge::iter::ListRange<'_> {
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

            if let Some(parent_path) = &node.parent_path_fallback {
                if parent_path == "." {
                    return ".".to_string();
                }
                return format!("{}/{}", node.parent_path_fallback.clone().unwrap_or(".".to_string()), node.name);
            }
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
        self.serialize_with_ext_resource_override(None, false)
    }

    // helper for the patchwork_resource_loader to serialize the scene at a given history ref for loading
    pub fn serialize_with_ext_resource_override(&self, history_ref: Option<&HistoryRef>, remove_uid_in_ext_resources: bool) -> String {

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
            if !remove_uid_in_ext_resources && let Some(uid) = &resource.uid {
                output.push_str(&format!(" uid=\"{}\"", uid));
            }
            let path = if let Some(history_ref) = history_ref {
                if !history_ref.is_valid() {
                    tracing::error!("History ref is not valid, can't rename dependencies!");
                    resource.path.clone()
                } else {
                    HistoryRefPath::make_path_string(history_ref, &resource.path).unwrap_or(resource.path.clone())
                }
            } else {
                resource.path.clone()
            };
            output.push_str(&format!(
                " path=\"{}\" id=\"{}\"]\n",
                path, resource.id
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

        // ensure blank line between connections and editable instances
        if self.connections.len() > 0 && self.editable_instances.len() > 0 {
            output.push('\n');
        }

        for path in self.editable_instances.iter() {
            output.push_str(&format!("[editable path=\"{}\"]\n", path));
        }
        output
    }

    fn serialize_node(&self, output: &mut String, node: &GodotNode, node_paths_visited: &mut HashMap<i32, i64>) {
		// name, type, parent, parent_id_path, owner, owner_uid_path, index, unique_id, node_paths, groups, instance_placeholder, instance
        output.push_str(&format!("[node name=\"{}\"", node.name));

        if let Some(TypeOrInstance::Type(t)) = &node.type_or_instance {
            output.push_str(&format!(" type=\"{}\"", t));
        }

        if let Some(parent_id) = node.parent_id {
            let parent_name = if self.root_node_id.is_some() && parent_id == self.root_node_id.unwrap() {
                ".".to_string()
            } else if let Some(parent_path) = &node.parent_path_fallback {
                parent_path.clone()
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
            output.push_str(&format!(" index=\"{}\"", index));
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

        if let Some(TypeOrInstance::Instance(i)) = &node.type_or_instance {
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

    pub fn get_ext_resource_path(&self, ext_resource_id: &String) -> Option<String> {
        self.ext_resources.get(ext_resource_id).map(|ext_resource| ext_resource.path.clone())
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

                    let resource_type = match heading.get("type").cloned() {
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
                        resource_type: scene_metadata.as_ref().map(|s| s.resource_type.clone()).unwrap_or("".to_string()),
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
                        None => -1
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
                        Some(TypeOrInstance::Type(unquote(&type_value)))
                    } else if let Some(instance_value) = heading.get("instance") {
                        Some(TypeOrInstance::Instance(unquote(instance_value)))
                    } else {
                        None
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
                        parent_path_fallback: parent_path.clone(),
						parent_id_path,
                        owner: heading.get("owner").cloned().map(|o| unquote(&o)),
						owner_uid_path,
                        index: heading.get("index").and_then(|i| unquote(i).parse::<i64>().ok()),
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

                                node.parent_path_fallback = None;
								Some(*parent_id)
							}
							None => {
                                // if we have a parent_id_path, get the first element
                                if let Some(parent_id_path) = &node.parent_id_path && !parent_id_path.is_empty() {
                                    let parent_id = parent_id_path[0];
                                    nodes.get_mut(&parent_id).unwrap().child_node_ids.push(node.id);
                                    Some(parent_id)
                                } else {
                                    return Err(format!(
                                        "Can't find parent node \"{}\" for node \"{}\"",
                                        parent_path, name
                                    ))
                                }
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


#[cfg(test)]
mod tests {
    use super::*;

    const INDEX_TEST: &str = r#"[gd_scene format=4 uid="uid://g64l65moc1sx"]
[ext_resource type="PackedScene" uid="uid://iu2q66clupc6" path="res://scenes/game_elements/characters/player/player.tscn" id="8_ukjsk"]

[node name="Minijuego2" type="Node2D" unique_id=1172299557]
y_sort_enabled = true

[node name="Player" parent="." unique_id=894731746 instance=ExtResource("8_ukjsk")]
y_sort_enabled = true
position = Vector2(435.00003, 347)
scale = Vector2(0.75, 0.75)

[node name="PlayerSprite" parent="Player" index="2" unique_id=1785485617]
position = Vector2(-2.6667075, -73.333336)

[node name="CollisionShape2D" parent="Player/PlayerInteraction/InteractZone" parent_id_path=PackedInt32Array(894731746, 888605377) index="0" unique_id=255765935]
position = Vector2(53.333294, -73.333336)
"#;
    #[test]
    fn test_unquote() {
        assert_eq!(unquote(&String::from("\"foo\"")), "foo");
        assert_eq!(unquote(&String::from("foo")), "foo");
    }

    #[test]
    fn test_scene() {
        // parse and re-serialize RAW_STRING
        let source = INDEX_TEST.to_string();
        let scene = parse_scene(&source).expect("parse should succeed");
        let serialized = scene.serialize();
        let round_trip = parse_scene(&serialized).expect("re-parse of serialized output should succeed");
        // println!("{}", serialized);
        assert_eq!(scene.uid, round_trip.uid);
        assert_eq!(scene.nodes.len(), round_trip.nodes.len());
        assert_eq!(source, serialized);
    }

    const COMPLEX_SCENE: &str = r#"[gd_scene format=4 uid="uid://g64l65moc1sx"]

[ext_resource type="TileSet" uid="uid://4ewcvb1sibsf" path="res://scenes/quests/story_quests/after_the_tremor/3_combat/2_cinematicacombate/tile/tilestaller.tres" id="1_8drhf"]
[ext_resource type="Shader" uid="uid://dmm7vum4u3k5e" path="res://scenes/quests/story_quests/after_the_tremor/0_intro/0_intro_part6/night.gdshader" id="2_ps5ij"]
[ext_resource type="Script" uid="uid://x1mxt6bmei2o" path="res://scenes/ui_elements/cinematic/cinematic.gd" id="3_a1j0i"]
[ext_resource type="Resource" uid="uid://x8dfau2t48gw" path="res://scenes/quests/story_quests/after_the_tremor/2_sequence_puzzle/2_tallerjuego/dialogojuego2.dialogue" id="4_ilpog"]
[ext_resource type="PackedScene" uid="uid://ipvcfv2g0oi1" path="res://scenes/game_elements/characters/npcs/talker/talker.tscn" id="5_8cyi3"]
[ext_resource type="Resource" uid="uid://dxtaa5oeofdxh" path="res://scenes/quests/story_quests/after_the_tremor/2_sequence_puzzle/2_tallerjuego/dialogodecharlie.dialogue" id="6_8cyi3"]
[ext_resource type="SpriteFrames" uid="uid://dbe6l18tss360" path="res://scenes/quests/story_quests/after_the_tremor/charlie_components/after_the_tremor_charlie.tres" id="7_321x3"]
[ext_resource type="PackedScene" uid="uid://iu2q66clupc6" path="res://scenes/game_elements/characters/player/player.tscn" id="8_ukjsk"]
[ext_resource type="SpriteFrames" uid="uid://dkcv15as5d6gw" path="res://scenes/quests/story_quests/after_the_tremor/bryan_components/after_the_tremor_bryan.tres" id="9_cbqvq"]
[ext_resource type="PackedScene" uid="uid://cfcgrfvtn04yp" path="res://scenes/ui_elements/hud/hud.tscn" id="10_8cyi3"]
[ext_resource type="PackedScene" uid="uid://fuhl3l6gxq5k" path="res://scenes/game_elements/props/collectible_item/collectible_item.tscn" id="11_gek8f"]
[ext_resource type="Script" uid="uid://bgmwplmj3bfls" path="res://scenes/globals/game_state/inventory/inventory_item.gd" id="12_321x3"]
[ext_resource type="Resource" uid="uid://dtqascp25vdgx" path="res://scenes/quests/story_quests/after_the_tremor/2_sequence_puzzle/2_tallerjuego/dialogofinal.dialogue" id="13_ea4l7"]
[ext_resource type="Script" uid="uid://c68oh8dtr21ti" path="res://scenes/game_logic/sequence_puzzle.gd" id="14_0n12u"]
[ext_resource type="PackedScene" uid="uid://b8sok264erfoc" path="res://scenes/game_elements/props/sequence_puzzle_object/sequence_puzzle_object.tscn" id="15_ea4l7"]
[ext_resource type="SpriteFrames" uid="uid://bosdk18pjhyo0" path="res://scenes/quests/story_quests/after_the_tremor/2_sequence_puzzle/botonfuente.tres" id="16_8drhf"]
[ext_resource type="AudioStream" uid="uid://cwlq7rc4rsvw" path="res://scenes/quests/story_quests/after_the_tremor/music/sounds/button.mp3" id="17_2yj5h"]
[ext_resource type="SpriteFrames" uid="uid://ds8aolx6fmtvx" path="res://scenes/quests/story_quests/after_the_tremor/2_sequence_puzzle/botonarco.tres" id="18_ls33p"]
[ext_resource type="SpriteFrames" uid="uid://c72utxw4qh4ui" path="res://scenes/quests/story_quests/after_the_tremor/2_sequence_puzzle/botonarboll.tres" id="19_fikee"]
[ext_resource type="SpriteFrames" uid="uid://do6ssglnwn60i" path="res://scenes/quests/story_quests/after_the_tremor/2_sequence_puzzle/botoncalabaza.tres" id="20_g31nb"]
[ext_resource type="Script" uid="uid://ccc78coj2b1li" path="res://scenes/game_logic/sequence_puzzle_step.gd" id="21_up7bl"]
[ext_resource type="PackedScene" uid="uid://be4o3ythda4cu" path="res://scenes/game_elements/props/sequence_puzzle_hint_sign/sequence_puzzle_hint_sign.tscn" id="22_0n12u"]
[ext_resource type="Script" uid="uid://cs3mackqqsjmr" path="res://scenes/quests/story_quests/after_the_tremor/2_sequence_puzzle/2_tallerjuego/reveal_tilemap.gd" id="23_ea4l7"]
[ext_resource type="AudioStream" uid="uid://buxupti22mp4y" path="res://scenes/quests/story_quests/after_the_tremor/music/minijuego2.ogg" id="24_8drhf"]
[ext_resource type="Script" uid="uid://d31cxd03praji" path="res://scenes/quests/story_quests/after_the_tremor/3_combat/scripts/MusicFix.gd" id="25_ls33p"]

[sub_resource type="ShaderMaterial" id="ShaderMaterial_famss"]
shader = ExtResource("2_ps5ij")
shader_parameter/enable_base_dark = true
shader_parameter/darkness = 0.150000007125
shader_parameter/vignette_strength = 5.0000002375
shader_parameter/vignette_size = 0.46100001863762
shader_parameter/enable_desat = true
shader_parameter/desaturation = 0.4
shader_parameter/enable_noise = true
shader_parameter/noise_intensity = 0.10000000475
shader_parameter/grain_amount = 0.250000011875

[sub_resource type="RectangleShape2D" id="RectangleShape2D_8cyi3"]
size = Vector2(80, 162.66667)

[sub_resource type="CapsuleShape2D" id="CapsuleShape2D_p26tb"]
radius = 21.333328
height = 74.666626

[sub_resource type="Resource" id="Resource_cbqvq"]
script = ExtResource("12_321x3")
metadata/_custom_type_script = "uid://bgmwplmj3bfls"

[node name="Minijuego2" type="Node2D" unique_id=1172299557]
y_sort_enabled = true

[node name="TileMapLayers" type="Node2D" parent="." unique_id=208114943]

[node name="Grass" type="TileMapLayer" parent="TileMapLayers" unique_id=798335763]
z_index = -1
tile_map_data = PackedByteArray("AAANAAcAFAAAAAAAAAANAAYAFAAAAAAAAAANAAUAFAAAAAAAAAANAAQAFAADABwAAAANAAMAFAABAAAAAAANAAIAFAAAAAAAAAANAAEAFAAAAAAAAAAMAAcAFAAAAAAAAAAMAAYAFAAAAAAAAAAMAAUAFAAAAAAAAAAMAAQAFAADABwAAAAMAAMAFAABAAAAAAAMAAIAFAAAAAAAAAAMAAEAFAAAAAAAAAAKAAcAFAADABsAAAAKAAYAFAADABsAAAAKAAUAFAADABwAAGAKAAQAFAAFAB0AAAAKAAMAFAABAAAAAAAKAAIAFAAHAB0AAAAJAAcAFAABAAAAAAAJAAYAFAABAAAAAAAJAAUAFAABAAAAAAAJAAQAFAABAAAAAAAJAAMAFAABAAAAAAAJAAIAFAABAAAAAAAIAAcAFAABAAAAAAAIAAYAFAABAAAAAAAIAAUAFAABAAAAAAAIAAQAFAABAAEAAAAIAAMAFAABAAAAAAAIAAIAFAABAAAAAAAHAAcAFAABAAAAAAAHAAYAFAABAAAAAAAHAAUAFAABAAAAAAAHAAQAFAABAAEAAAAHAAMAFAABAAAAAAAHAAIAFAABAAAAAAAGAAcAFAABAAAAAAAGAAYAFAABAAAAAAAGAAUAFAABAAAAAAAGAAQAFAABAAAAAAAGAAMAFAABAAAAAAAGAAIAFAABAAAAAAAFAAcAFAABAAAAAAAFAAYAFAABAAAAAAAFAAUAFAABAAAAAAAFAAQAFAACACkAAAAFAAMAFAABAAAAAAAFAAIAFAABAAAAAAAFAAEAFAABAAEAAAAEAAcAFAABAAAAAAAEAAYAFAABAAAAAAAEAAQAFAABAAAAAAAEAAMAFAABAAAAAAAEAAIAFAABAAAAAAAEAAEAFAABAAAAAAADAAQAFAABAAAAAAADAAMAFAABAAAAAAADAAIAFAAGAB0AAAADAAEAFAADABoAAAACAAMAFAABAAAAAAACAAIAFAAHAB0AAAACAAEAFAAAAAAAAAABAAMAFAABAAAAAAABAAIAFAABAAAAAAABAAEAFAADAB0AAAACAAQAFAABAAAAAAABAAQAFAAEAB0AAAAAAAQAFAAAAAAAAAAHAAAAFAABAAAAAAAFAAAAFAABAAAAAAAIAAAAFAABAAAAAAAJAAAAFAABAAAAAAAMAAAAFAAAAAAAAAAKAAAAFAADABsAAAANAAAAFAAAAAAAAAAEAAAAFAABAAAAAAADAAAAFAADABoAAAACAAAAFAAAAAAAAAABAAAAFAAAAAAAAAAAAAAAFAAAAAAAAAAAAAEAFAAHAB0AAAAAAAIAFAABAAEAAAAAAAMAFAABAAAAAAAAAAgAFAAAAAAAAAABAAgAFAAAAAAAAAACAAgAFAAAAAAAAAADAAgAFAADABoAAAAEAAgAFAABAAAAAAAFAAgAFAABAAAAAAAGAAgAFAABAAAAAAAHAAgAFAABAAAAAAAIAAgAFAABAAAAAAAJAAgAFAABAAAAAAAKAAgAFAADABsAAAAMAAgAFAAAAAAAAAANAAgAFAAAAAAAAAAOAAgAFAAAAAAAAAAOAAcAFAAAAAAAAAAOAAYAFAAAAAAAAAAOAAUAFAAAAAAAAAAOAAQAFAADABwAAAAOAAMAFAABAAAAAAAOAAIAFAAGAB0AAAAOAAEAFAAAAAAAAAAOAAAAFAAAAAAAAAAAAAUAFAAAAAAAAAABAAUAFAAAAAAAAAACAAUAFAAAAAAAAAADAAUAFAAEAB0AAAAEAAUAFAABAAAAAAADAAYAFAADABoAAAACAAYAFAAAAAAAAAABAAYAFAAAAAAAAAAAAAYAFAAAAAAAAAAAAAcAFAAAAAAAAAABAAcAFAAAAAAAAAACAAcAFAAAAAAAAAADAAcAFAADABoAAAAGAAEAFAABAAAAAAAHAAEAFAABAAAAAAAIAAEAFAABAAAAAAAJAAEAFAABAAAAAAAKAAEAFAADABsAAAAPAAkAFAAAAAAAAAAPAAgAFAAAAAAAAAAPAAcAFAAAAAAAAAAPAAYAFAAAAAAAAAAPAAUAFAAAAAAAAAAPAAQAFAADABwAAAAPAAMAFAABAAAAAAAPAAIAFAABAAAAAAAPAAEAFAAAAAAAAAAPAAAAFAAAAAAAAAAPAP//FAAAAAAAAAAOAAkAFAAAAAAAAAAOAP//FAAAAAAAAAANAAkAFAAAAAAAAAANAP//FAAAAAAAAAAMAAkAFAAAAAAAAAAMAP//FAAAAAAAAAAKAAkAFAADABsAAAAKAP//FAADABsAAAAJAAkAFAABAAAAAAAJAP//FAABAAAAAAAIAAkAFAABAAAAAAAIAP//FAABAAAAAAAHAAkAFAABAAEAAAAHAP//FAABAAAAAAAGAAkAFAABAAAAAAAGAP//FAABAAAAAAAFAAkAFAABAAEAAAAFAP//FAABAAAAAAAEAAkAFAABAAAAAAAEAP//FAABAAAAAAADAAkAFAADABoAAAADAP//FAADABoAAAACAAkAFAAAAAAAAAACAP//FAAAAAAAAAABAAkAFAAAAAAAAAABAP//FAAAAAAAAAAAAAkAFAAAAAAAAAAAAP//FAAAAAAAAAD//wkAFAAAAAAAAAD//wgAFAAAAAAAAAD//wcAFAAAAAAAAAD//wYAFAAAAAAAAAD//wUAFAAAAAAAAAD//wQAFAAAAAAAAAD//wMAFAAEAB0AAAD//wIAFAABAAAAAAD//wEAFAABAAAAAAD//wAAFAADAB0AAAD/////FAAAAAAAAAAEAAoAFAABAAAAAAAEAAsAFAABAAAAAAAEAAwAFAABAAAAAAAEAA0AFAABAAAAAAAEAA4AFAABAAAAAAAEAA8AFAABAAAAAAAEABAAFAABAAAAAAAEABEAFAABAAAAAAAEABIAFAABAAAAAAAFAAoAFAABAAAAAAAFAAsAFAABAAAAAAAFAAwAFAABAAAAAAAFAA0AFAABAAAAAAAFAA4AFAABAAEAAAAFAA8AFAABAAAAAAAFABAAFAABAAAAAAAFABEAFAABAAAAAAAFABIAFAABAAAAAAAGAAAAFAABAAAAAAAGAAoAFAABAAAAAAAGAAsAFAABAAEAAAAGAAwAFAABAAAAAAAGAA0AFAABAAEAAAAGAA4AFAABAAAAAAAGAA8AFAABAAAAAAAGABAAFAABAAAAAAAGABEAFAABAAAAAAAGABIAFAABAAAAAAAHAAoAFAABAAAAAAAHAAsAFAABAAAAAAAHAAwAFAABAAAAAAAHAA0AFAABAAEAAAAHAA4AFAABAAAAAAAHAA8AFAABAAAAAAAHABAAFAABAAAAAAAHABEAFAABAAAAAAAHABIAFAABAAAAAAAIAAoAFAABAAAAAAAIAAsAFAABAAEAAAAIAA0AFAABAAAAAAAIAA4AFAABAAAAAAAIAA8AFAABAAAAAAAIABAAFAABAAAAAAAIABEAFAABAAAAAAAIABIAFAABAAAAAAAJAAoAFAABAAAAAAAJAAsAFAABAAAAAAAJAAwAFAABAAAAAAAJAA0AFAABAAAAAAAJAA4AFAABAAAAAAAJAA8AFAABAAAAAAAJABAAFAABAAAAAAAJABEAFAABAAAAAAAJABIAFAABAAAAAAAKAAoAFAADABsAAAAKAAsAFAADABsAAAAKAAwAFAADABsAAAAKAA0AFAADABsAAAAKAA4AFAADABsAAAAKAA8AFAADABsAAAAKABAAFAADABsAAAAKABEAFAADABsAAAAKABIAFAADABsAAAAKABMAFAADABsAAAAKABQAFAADABsAAAAKABUAFAADABsAAAAKABYAFAADABsAAAADAAoAFAADABoAAAADAAsAFAADABoAAAADAAwAFAAGAB0AAAADAA0AFAABAAAAAAADAA4AFAABAAAAAAADAA8AFAABAAAAAAADABAAFAAEAB0AAAADABEAFAADABoAAAADABIAFAADABoAAAADABMAFAADABoAAAADABQAFAADABoAAAADABUAFAADABoAAAADABYAFAADABoAAAAIAAwAFAABAAAAAAAEABMAFAABAAAAAAAFABMAFAABAAAAAAAFABQAFAABAAAAAAAGABQAFAABAAAAAAAHABUAFAABAAAAAAAIABYAFAABAAAAAAAJABYAFAABAAAAAAAJABUAFAABAAAAAAAJABQAFAABAAAAAAAJABMAFAABAAAAAAAIABMAFAABAAAAAAAHABMAFAABAAAAAAAGABMAFAABAAAAAAAHABQAFAABAAAAAAAIABQAFAABAAAAAAAIABUAFAABAAAAAAAGABUAFAABAAAAAAAEABQAFAABAAAAAAAEABUAFAABAAAAAAAEABYAFAABAAAAAAAFABYAFAABAAAAAAAGABYAFAABAAAAAAAHABYAFAABAAAAAAAFABUAFAABAAAAAAADAP7/FAAEABwAAAAEAP7/FAAFABwAAAAFAP7/FAAFABwAAAAGAP7/FAAFABwAAAAHAP7/FAAFABwAAAAIAP7/FAAFABwAAAAJAP7/FAAFABwAAAAKAP7/FAAGABwAAAADAPz/FAAEABsAAAADAP3/FAAEABsAAAAEAPz/FAAFABsAAAAEAP3/FAAFABsAAAAFAPz/FAAFABsAAAAFAP3/FAAFABsAAAAGAPz/FAAFABsAAAAGAP3/FAAFABsAAAAHAPz/FAAFABsAAAAHAP3/FAAFABsAAAAIAPz/FAAFABsAAAAIAP3/FAAFABsAAAAJAPz/FAAFABsAAAAJAP3/FAAFABsAAAAKAPz/FAAGABsAAAAKAP3/FAAGABsAAAADAPv/FAAEABoAAAAEAPv/FAAFABoAAAAFAPv/FAAGABoAAAAGAPv/FAAFABoAAAAHAPv/FAAFABoAAAAIAPv/FAAFABoAAAAJAPv/FAAFABoAAAAKAPv/FAAGABoAAAADAPr/FAAGABkAAAAEAPr/FAAGABkAAAAFAPr/FAAGABkAAAAGAPr/FAAGABkAAAAHAPr/FAAGABkAAAAIAPr/FAAGABkAAAAJAPr/FAAGABkAAAAKAPr/FAAGABkAAAALAPz/FAADAAQAAAALAP3/FAAAAAQAAAALAP7/FAAAAAUAAAACAPz/FAAFAAQAAAACAP3/FAAHAAQAAAACAP7/FAAHAAUAAAACAPv/FAAHABkAAAACAPr/FAAEABkAAAALAPr/FAADABkAAAALAPv/FAAFABkAAAD3////FAABAAAAAAD3/wAAFAADABwAAAD3/wEAFAAAAAAAAAD3/wIAFAAAAAAAAAD3/wMAFAAAAAAAAAD3/wQAFAAAAAAAAAD3/wUAFAAAAAAAAAD3/wYAFAAAAAAAAAD3/wcAFAAAAAAAAAD3/wgAFAAAAAAAAAD3/wkAFAAAAAAAAAD3/woAFAAAAAAAAAD3/wsAFAAAAAAAAAD3/wwAFAADAB0AAAD3/w0AFAABAAAAAAD3/w4AFAABAAAAAAD3/w8AFAABAAAAAAD3/xAAFAADABwAAAD3/xEAFAAAAAAAAAD4/wAAFAADABwAAAD4/wEAFAAAAAAAAAD4/wIAFAAAAAAAAAD4/wMAFAAAAAAAAAD4/wQAFAAAAAAAAAD4/wUAFAAAAAAAAAD4/wYAFAAAAAAAAAD4/wcAFAAAAAAAAAD4/wgAFAAAAAAAAAD4/wkAFAAAAAAAAAD4/woAFAAAAAAAAAD4/wsAFAAAAAAAAAD4/wwAFAADAB0AAAD4/w0AFAABAAAAAAD4/w4AFAABAAAAAAD4/w8AFAABAAAAAAD4/xAAFAADABwAAAD4/xEAFAAAAAAAAAD5////FAABAAAAAAD5/wAAFAAEAB0AAAD5/wEAFAAAAAAAAAD5/wIAFAAAAAAAAAD5/wMAFAAAAAAAAAD5/wQAFAAAAAAAAAD5/wUAFAAAAAAAAAD5/wYAFAAAAAAAAAD5/wcAFAAAAAAAAAD5/wgAFAAAAAAAAAD5/wkAFAAAAAAAAAD5/woAFAAAAAAAAAD5/wsAFAAAAAAAAAD5/wwAFAADAB0AAAD5/w0AFAABAAEAAAD5/w4AFAACACsAAAD5/w8AFAABAAAAAAD5/xAAFAADABwAAAD5/xEAFAAAAAAAAAD6////FAACACkAAAD6/wAAFAABAAAAAAD6/wEAFAAAAAAAAAD6/wIAFAAAAAAAAAD6/wMAFAAAAAAAAAD6/wQAFAAAAAAAAAD6/wUAFAAAAAAAAAD6/wYAFAAAAAAAAAD6/wcAFAAAAAAAAAD6/wgAFAAAAAAAAAD6/wkAFAAAAAAAAAD6/woAFAAAAAAAAAD6/wsAFAAAAAAAAAD6/wwAFAADAB0AAAD6/w0AFAABAAAAAAD6/w4AFAABAAAAAAD6/w8AFAABAAAAAAD6/xAAFAADABwAAAD6/xEAFAAAAAAAAAD7////FAABAAAAAAD7/wAAFAABAAAAAAD7/wEAFAAEAB0AAAD7/wIAFAAAAAAAAAD7/wMAFAAAAAAAAAD7/wQAFAAAAAAAAAD7/wUAFAAAAAAAAAD7/wYAFAAAAAAAAAD7/wcAFAAAAAAAAAD7/wgAFAAAAAAAAAD7/wkAFAAAAAAAAAD7/woAFAAAAAAAAAD7/wsAFAAAAAAAAAD7/wwAFAADAB0AAAD7/w0AFAABAAAAAAD7/w4AFAABAAAAAAD7/w8AFAABAAAAAAD7/xAAFAADABwAAAD7/xEAFAAAAAAAAAD8////FAAHAB0AAAD8/wAAFAABAAEAAAD8/wEAFAABAAAAAAD8/wIAFAAAAAAAAAD8/wMAFAAAAAAAAAD8/wQAFAAAAAAAAAD8/wUAFAAAAAAAAAD8/wYAFAAAAAAAAAD8/wcAFAAAAAAAAAD8/wgAFAAAAAAAAAD8/wkAFAAAAAAAAAD8/woAFAAAAAAAAAD8/wsAFAAAAAAAAAD8/wwAFAADAB0AAAD8/w0AFAABAAAAAAD8/w4AFAABAAAAAAD8/w8AFAABAAEAAAD8/xAAFAADABwAAAD8/xEAFAAAAAAAAAD9////FAADAB0AAAD9/wAAFAABAAAAAAD9/wEAFAABAAAAAAD9/wIAFAAEAB0AAAD9/wMAFAAAAAAAAAD9/wQAFAAAAAAAAAD9/wUAFAAAAAAAAAD9/wYAFAAAAAAAAAD9/wcAFAAAAAAAAAD9/wgAFAAAAAAAAAD9/wkAFAAAAAAAAAD9/woAFAAAAAAAAAD9/wsAFAAAAAAAAAD9/wwAFAADAB0AAAD9/w0AFAABAAAAAAD9/w4AFAABAAAAAAD9/w8AFAABAAAAAAD9/xAAFAADABwAAAD9/xEAFAAAAAAAAAD+////FAAAAAAAAAD+/wAAFAAHAB0AAAD+/wEAFAABAAAAAAD+/wIAFAABAAAAAAD+/wMAFAAAAAAAAAD+/wQAFAAAAAAAAAD+/wUAFAAAAAAAAAD+/wYAFAAAAAAAAAD+/wcAFAAAAAAAAAD+/wgAFAAAAAAAAAD+/wkAFAAAAAAAAAD+/woAFAAAAAAAAAD+/wsAFAAAAAAAAAD+/wwAFAADAB0AAAD+/w0AFAABAAAAAAD+/w4AFAABAAAAAAD+/w8AFAABAAAAAAD+/xAAFAADABwAAAD+/xEAFAAAAAAAAAD//woAFAAAAAAAAAD//wsAFAAAAAAAAAD//wwAFAADAB0AAAD//w0AFAABAAAAAAD//w4AFAACACkAAAD//w8AFAABAAEAAAD//xAAFAADABwAAAD//xEAFAAAAAAAAAAAAAoAFAAAAAAAAAAAAAsAFAAAAAAAAAAAAAwAFAADAB0AAAAAAA0AFAABAAAAAAAAAA4AFAABAAAAAAAAAA8AFAABAAAAAAAAABAAFAADABwAAAAAABEAFAAAAAAAAAABAAoAFAAAAAAAAAABAAsAFAAAAAAAAAABAAwAFAADAB0AAAABAA0AFAABAAAAAAABAA4AFAABAAAAAAABAA8AFAABAAAAAAABABAAFAADABwAAAABABEAFAAAAAAAAAACAAoAFAAAAAAAAAACAAsAFAAAAAAAAAACAAwAFAADAB0AAAACAA0AFAABAAAAAAACAA4AFAABAAAAAAACAA8AFAABAAAAAAACABAAFAADABwAAAACABEAFAAAAAAAAAD3/xIAFAAAAAAAAAD3/xMAFAAAAAAAAAD3/xQAFAAAAAAAAAD3/xUAFAAAAAAAAAD3/xYAFAAAAAAAAAD4/xIAFAAAAAAAAAD4/xMAFAAAAAAAAAD4/xQAFAAAAAAAAAD4/xUAFAAAAAAAAAD4/xYAFAAAAAAAAAD5/xIAFAAAAAAAAAD5/xMAFAAAAAAAAAD5/xQAFAAAAAAAAAD5/xUAFAAAAAAAAAD5/xYAFAAAAAAAAAD6/xIAFAAAAAAAAAD6/xMAFAAAAAAAAAD6/xQAFAAAAAAAAAD6/xUAFAAAAAAAAAD6/xYAFAAAAAAAAAD7/xIAFAAAAAAAAAD7/xMAFAAAAAAAAAD7/xQAFAAAAAAAAAD7/xUAFAAAAAAAAAD7/xYAFAAAAAAAAAD8/xIAFAAAAAAAAAD8/xMAFAAAAAAAAAD8/xQAFAAAAAAAAAD8/xUAFAAAAAAAAAD8/xYAFAAAAAAAAAD9/xIAFAAAAAAAAAD9/xMAFAAAAAAAAAD9/xQAFAAAAAAAAAD9/xUAFAAAAAAAAAD9/xYAFAAAAAAAAAD+/xIAFAAAAAAAAAD+/xMAFAAAAAAAAAD+/xQAFAAAAAAAAAD+/xUAFAAAAAAAAAD+/xYAFAAAAAAAAAD//xIAFAAAAAAAAAD//xMAFAAAAAAAAAD//xQAFAAAAAAAAAD//xUAFAAAAAAAAAD//xYAFAAAAAAAAAAAABIAFAAAAAAAAAAAABMAFAAAAAAAAAAAABQAFAAAAAAAAAAAABUAFAAAAAAAAAAAABYAFAAAAAAAAAABABIAFAAAAAAAAAABABMAFAAAAAAAAAABABQAFAAAAAAAAAABABUAFAAAAAAAAAABABYAFAAAAAAAAAACABIAFAAAAAAAAAACABMAFAAAAAAAAAACABQAFAAAAAAAAAACABUAFAAAAAAAAAACABYAFAAAAAAAAAALAP//FAAAAAAAAAALAAAAFAAAAAAAAAALAAEAFAAAAAAAAAALAAIAFAAAAAAAAAALAAMAFAABAAAAAAALAAQAFAADABwAAAALAAUAFAAAAAAAAAALAAYAFAAAAAAAAAALAAcAFAAAAAAAAAALAAgAFAAAAAAAAAALAAkAFAAAAAAAAAALAAoAFAAAAAAAAAALAAsAFAAAAAAAAAALAAwAFAAAAAAAAAALAA0AFAACACEAAAALAA4AFAACACIAAAALAA8AFAACACMAAAALABAAFAAAAAAAAAALABEAFAAAAAAAAAALABIAFAAAAAAAAAALABMAFAAAAAAAAAALABQAFAAAAAAAAAALABUAFAAAAAAAAAALABYAFAAAAAAAAAAMAAoAFAAAAAAAAAAMAAsAFAAAAAAAAAAMAAwAFAAAAAAAAAAMAA0AFAADACEAAAAMAA4AFAADACIAAAAMAA8AFAADACMAAAAMABAAFAAAAAAAAAAMABEAFAAAAAAAAAAMABIAFAAAAAAAAAAMABMAFAAAAAAAAAAMABQAFAAAAAAAAAAMABUAFAAAAAAAAAAMABYAFAAAAAAAAAANAAoAFAAAAAAAAAANAAsAFAAAAAAAAAANAAwAFAAAAAAAAAANAA0AFAAEACEAAAANAA4AFAAEACIAAAANAA8AFAAEACMAAAANABAAFAAAAAAAAAANABEAFAAAAAAAAAANABIAFAABACEAABANABMAFAABACIAABANABQAFAAAAAAAAAANABUAFAAAAAAAAAANABYAFAAAAAAAAAAOAAoAFAAAAAAAAAAOAAsAFAAAAAAAAAAOAAwAFAAAAAAAAAAOAA0AFAAAAAAAAAAOAA4AFAAFACIAAAAOAA8AFAAFACMAAAAOABAAFAAAAAAAAAAOABEAFAAAAAAAAAAOABIAFAAAACEAABAOABMAFAAAACIAABAOABQAFAAAAAAAAAAOABUAFAAAAAAAAAAOABYAFAAAAAAAAAAPAAoAFAAAAAAAAAAPAAsAFAAAAAAAAAAPAAwAFAAAAAAAAAAPAA0AFAAAAAAAAAAPAA4AFAAAAAAAAAAPAA8AFAAAAAAAAAAPABAAFAAAACoAAAAPABEAFAAAACoAAAAPABIAFAAAACoAAAAPABMAFAAAAAAAAAAPABQAFAAAAAAAAAAPABUAFAAAAAAAAAAPABYAFAAAAAAAAAAQAP//FAAAAAAAAAAQAAAAFAAAAAAAAAAQAAEAFAAEAB0AACAQAAIAFAABAAAAAAAQAAMAFAAEACkAAAAQAAQAFAADABwAAAAQAAUAFAAAAAAAAAAQAAYAFAAAAAAAAAAQAAcAFAAAAAAAAAAQAAgAFAAAAAAAAAAQAAkAFAAAAAAAAAAQAAoAFAAAAAAAAAAQAAsAFAAAAAAAAAAQAAwAFAAAAAAAAAAQAA0AFAAAAAAAAAAQAA4AFAAAAAAAAAAQAA8AFAABACkAAAAQABAAFAAFACoAAAAQABEAFAAFACoAAAAQABIAFAAFACoAAAAQABMAFAABACsAAAAQABQAFAAAAAAAAAAQABUAFAAAAAAAAAAQABYAFAAAAAAAAAARAP//FAAAAAAAAAARAAAAFAADAB0AAAARAAEAFAABAAAAAAARAAIAFAABAAEAAAARAAMAFAAEACkAAAARAAQAFAADABwAAAARAAUAFAAAAAAAAAARAAYAFAAAAAAAAAARAAcAFAAAAAAAAAARAAgAFAAAAAAAAAARAAkAFAAAAAAAAAARAAoAFAAAAAAAAAARAAsAFAAAAAAAAAARAAwAFAAAAAAAAAARAA0AFAAAAAAAAAARAA4AFAAAAAAAAAARAA8AFAABACkAAAARABAAFAABACoAAAARABEAFAABACoAAAARABIAFAABACoAAAARABMAFAABACsAAAARABQAFAAAAAAAAAARABUAFAAAAAAAAAARABYAFAAAAAAAAAASAP//FAAAAAAAAAASAAAAFAAGAB0AAAASAAEAFAABAAAAAAASAAIAFAABAAAAAAASAAMAFAAEACkAAAASAAQAFAADABwAAAASAAUAFAAAAAAAAAASAAYAFAAAAAAAAAASAAcAFAAAAAAAAAASAAgAFAAAAAAAAAASAAkAFAAAAAAAAAASAAoAFAAAAAAAAAASAAsAFAAAAAAAAAASAAwAFAAAAAAAAAASAA0AFAAAAAAAAAASAA4AFAAAAAAAAAASAA8AFAABACkAAAASABAAFAADACkAAAASABEAFAADACkAAAASABIAFAABACoAAAASABMAFAABACsAAAASABQAFAAAAAAAAAASABUAFAAAAAAAAAASABYAFAAAAAAAAAATAP//FAADAB0AAAATAAAAFAABAAAAAAATAAEAFAABAAAAAAATAAIAFAABAAAAAAATAAMAFAAEACkAAAATAAQAFAADABwAAAATAAUAFAAAAAAAAAATAAYAFAAAAAAAAAATAAcAFAAAAAAAAAATAAgAFAAAAAAAAAATAAkAFAAAAAAAAAATAAoAFAAAAAAAAAATAAsAFAAAAAAAAAATAAwAFAAAAAAAAAATAA0AFAAAAAAAAAATAA4AFAAAAAAAAAATAA8AFAABACkAAAATABAAFAABACoAAAATABEAFAADACkAAAATABIAFAADACkAAAATABMAFAABACsAAAATABQAFAAAAAAAAAATABUAFAAAAAAAAAATABYAFAAAAAAAAAAUAP//FAAGAB0AAAAUAAAAFAABAAAAAAAUAAEAFAABAAAAAAAUAAIAFAABAAEAAAAUAAMAFAAEACkAAAAUAAQAFAADABwAAAAUAAUAFAAAAAAAAAAUAAYAFAAAAAAAAAAUAAcAFAAAAAAAAAAUAAgAFAAAAAAAAAAUAAkAFAAAAAAAAAAUAAoAFAAAAAAAAAAUAAsAFAAAAAAAAAAUAAwAFAAAAAAAAAAUAA0AFAAAAAAAAAAUAA4AFAAAAAAAAAAUAA8AFAABACkAAAAUABAAFAADACkAAAAUABEAFAABACoAAAAUABIAFAABACoAAAAUABMAFAABACsAAAAUABQAFAAAAAAAAAAUABUAFAAAAAAAAAAUABYAFAAAAAAAAAAVAP//FAABAAAAAAAVAAAAFAACACkAAAAVAAEAFAABAAAAAAAVAAIAFAABAAAAAAAVAAMAFAABAAAAAAAVAAQAFAADABwAAAAVAAUAFAAAAAAAAAAVAAYAFAAAAAAAAAAVAAcAFAAAAAAAAAAVAAgAFAAAAAAAAAAVAAkAFAAAAAAAAAAVAAoAFAAAAAAAAAAVAAsAFAAAAAAAAAAVAAwAFAAAAAAAAAAVAA0AFAAAAAAAAAAVAA4AFAAAAAAAAAAVAA8AFAABACkAAAAVABAAFAABACoAAAAVABEAFAABACoAAAAVABIAFAABACoAAAAVABMAFAABACsAAAAVABQAFAAAAAAAAAAVABUAFAAAAAAAAAAVABYAFAAAAAAAAAAWAP//FAABAAAAAAAWAAAAFAABAAAAAAAWAAEAFAABAAAAAAAWAAIAFAABAAAAAAAWAAQAFAAAAAAAAAAWAAUAFAAAAAAAAAAWAAYAFAAAAAAAAAAWAAcAFAAAAAAAAAAWAAgAFAAAAAAAAAAWAAkAFAAAAAAAAAAWAAoAFAAAAAAAAAAWAAsAFAAAAAAAAAAWAAwAFAAAAAAAAAAWAA0AFAAAAAAAAAAWAA4AFAAAAAAAAAAWAA8AFAABACkAAAAWABAAFAAEACoAAAAWABEAFAAEACoAAAAWABIAFAAEACoAAAAWABMAFAABACsAAAAWABQAFAAAAAAAAAAWABUAFAAAAAAAAAAWABYAFAAAAAAAAAAXAP//FAABAAAAAAAXAAAAFAABAAEAAAAXAAEAFAABAAAAAAAXAAIAFAAFAB0AAAAXAAMAFAAAAAAAAAAXAAQAFAAAAAAAAAAXAAUAFAAAAAAAAAAXAAYAFAAAAAAAAAAXAAcAFAAAAAAAAAAXAAgAFAAAAAAAAAAXAAkAFAAAAAAAAAAXAAoAFAAAAAAAAAAXAAsAFAAAAAAAAAAXAAwAFAAAAAAAAAAXAA0AFAAAAAAAAAAXAA4AFAAAAAAAAAAXAA8AFAAAAAAAAAAXABAAFAACACoAAAAXABEAFAACACoAAAAXABIAFAACACoAAAAXABQAFAAAAAAAAAAXABUAFAAAAAAAAAAXABYAFAAAAAAAAAACAPD/FAACABgAAAACAPH/FAACABgAAAACAPL/FAACABgAAAACAPP/FAACABgAAAACAPT/FAACABgAAAACAPX/FAACABgAAAACAPb/FAACABgAAAACAPf/FAACABgAAAACAPj/FAACABgAAAACAPn/FAACABgAAAADAPD/FAAEABcAAAADAPH/FAAEABcAAAADAPL/FAADABcAAAADAPP/FAADABcAAAADAPT/FAADABcAAAADAPX/FAAEABcAAAADAPb/FAAEABcAAAADAPf/FAADABcAAAADAPj/FAADABcAAAADAPn/FAAEABcAAAAEAPD/FAAEABcAAAAEAPH/FAAEABcAAAAEAPL/FAADABcAAAAEAPP/FAADABcAAAAEAPT/FAAEABcAAAAEAPX/FAADABcAAAAEAPb/FAAEABcAAAAEAPf/FAAEABcAAAAEAPj/FAADABcAAAAEAPn/FAADABcAAAAFAPD/FAAEABcAAAAFAPH/FAADABcAAAAFAPL/FAAEABcAAAAFAPP/FAADABcAAAAFAPT/FAAEABcAAAAFAPX/FAADABcAAAAFAPb/FAAEABcAAAAFAPf/FAADABcAAAAFAPj/FAADABcAAAAFAPn/FAAEABcAAAAGAPD/FAAEABcAAAAGAPH/FAADABcAAAAGAPL/FAADABcAAAAGAPP/FAADABcAAAAGAPT/FAADABcAAAAGAPX/FAADABcAAAAGAPb/FAADABcAAAAGAPf/FAAEABcAAAAGAPj/FAADABcAAAAGAPn/FAAEABcAAAAHAPD/FAADABcAAAAHAPH/FAADABcAAAAHAPL/FAAEABcAAAAHAPP/FAAEABcAAAAHAPT/FAADABcAAAAHAPX/FAADABcAAAAHAPb/FAAEABcAAAAHAPf/FAAEABcAAAAHAPj/FAAEABcAAAAHAPn/FAAEABcAAAAIAPD/FAADABcAAAAIAPH/FAAEABcAAAAIAPL/FAADABcAAAAIAPP/FAADABcAAAAIAPT/FAAEABcAAAAIAPX/FAAEABcAAAAIAPb/FAADABcAAAAIAPf/FAADABcAAAAIAPj/FAAEABcAAAAIAPn/FAADABcAAAAJAPD/FAADABcAAAAJAPH/FAAEABcAAAAJAPL/FAAEABcAAAAJAPP/FAADABcAAAAJAPT/FAAEABcAAAAJAPX/FAAEABcAAAAJAPb/FAAEABcAAAAJAPf/FAADABcAAAAJAPj/FAADABcAAAAJAPn/FAADABcAAAAKAPD/FAADABcAAAAKAPH/FAAEABcAAAAKAPL/FAAEABcAAAAKAPP/FAAEABcAAAAKAPT/FAAEABcAAAAKAPX/FAAEABcAAAAKAPb/FAAEABcAAAAKAPf/FAAEABcAAAAKAPj/FAAEABcAAAAKAPn/FAAEABcAAAALAPD/FAAAABgAAAALAPH/FAAAABgAAAALAPL/FAAAABgAAAALAPP/FAAAABgAAAALAPT/FAAAABgAAAALAPX/FAAAABgAAAALAPb/FAAAABgAAAALAPf/FAAAABgAAAALAPj/FAAAABgAAAALAPn/FAAAABgAAAD8/+7/FAAAAAAAAAD8/+//FAAAAAAAAAD8//D/FAAAAAAAAAD8//H/FAAAAAAAAAD8//L/FAAAAAAAAAD8//P/FAAAAAAAAAD8//T/FAAAAAAAAAD8//X/FAAAAAAAAAD8//b/FAAAAAAAAAD8//f/FAAAAAAAAAD8//j/FAAAAAAAAAD8//n/FAAAAAAAAAD8//r/FAAAAAAAAAD9/+7/FAAAAAAAAAD9/+//FAAAAAAAAAD9//D/FAAAAAAAAAD9//H/FAAAAAAAAAD9//L/FAAAAAAAAAD9//P/FAAAAAAAAAD9//T/FAAAAAAAAAD9//X/FAAAAAAAAAD9//b/FAAAAAAAAAD9//f/FAAAAAAAAAD9//j/FAAAAAAAAAD9//n/FAAAAAAAAAD9//r/FAAAAAAAAAD+/+7/FAAAAAAAAAD+/+//FAAAAAAAAAD+//D/FAAAAAAAAAD+//H/FAAAAAAAAAD+//L/FAAAAAAAAAD+//P/FAAAAAAAAAD+//T/FAAAAAAAAAD+//X/FAAAAAAAAAD+//b/FAAAAAAAAAD+//f/FAAAAAAAAAD+//j/FAAAAAAAAAD+//n/FAAAAAAAAAD+//r/FAAAAAAAAAD//+7/FAAAAAAAAAD//+//FAAAAAAAAAD///D/FAAAAAAAAAD///H/FAAAAAAAAAD///L/FAAAAAAAAAD///P/FAAAAAAAAAD///T/FAAAAAAAAAD///X/FAAAAAAAAAD///b/FAAAAAAAAAD///f/FAAAAAAAAAD///j/FAAAAAAAAAD///n/FAAAAAAAAAD///r/FAAAAAAAAAAAAO7/FAAAAAAAAAAAAO//FAAAAAAAAAAAAPD/FAAAAAAAAAAAAPH/FAAAAAAAAAAAAPL/FAAAAAAAAAAAAPP/FAAAAAAAAAAAAPT/FAAAAAAAAAAAAPX/FAAAAAAAAAAAAPb/FAAAAAAAAAAAAPf/FAAAAAAAAAAAAPj/FAAAAAAAAAAAAPn/FAAAAAAAAAAAAPr/FAAAAAAAAAABAO7/FAAAABcAAAABAO//FAAAABgAAAABAPD/FAAAABgAAAABAPH/FAAAABgAAAABAPL/FAAAABgAAAABAPP/FAAAABgAAAABAPT/FAAAABgAAAABAPX/FAAAABgAAAABAPb/FAAAABgAAAABAPf/FAAAABgAAAABAPj/FAAAABgAAAABAPn/FAAAABgAAAABAPr/FAAAABgAAAACAO7/FAACABcAAAACAO//FAACABgAAAADAO7/FAADABcAAAADAO//FAAEABcAAAAEAO7/FAADABcAAAAEAO//FAADABcAAAAFAO7/FAADABcAAAAFAO//FAAEABcAAAAGAO7/FAADABcAAAAGAO//FAADABcAAAAHAO7/FAADABcAAAAHAO//FAADABcAAAAIAO7/FAADABcAAAAIAO//FAAEABcAAAAJAO7/FAAEABcAAAAJAO//FAAEABcAAAAKAO7/FAADABcAAAAKAO//FAAEABcAAAALAO7/FAAAABgAAAALAO//FAAAABgAAAAMAO7/FAAGABcAAAAMAO//FAADABcAAAAMAPD/FAADABcAAAAMAPH/FAADABcAAAAMAPL/FAADABcAAAAMAPP/FAADABcAAAAMAPT/FAADABcAAAAMAPX/FAADABcAAAAMAPb/FAADABcAAAAMAPf/FAADABcAAAAMAPj/FAADABcAAAAMAPn/FAADABcAAAAMAPr/FAADABcAAAANAO7/FAABABcAAAANAO//FAADABcAAAANAPD/FAADABcAAAANAPH/FAADABcAAAANAPL/FAADABcAAAANAPP/FAADABcAAAANAPT/FAADABcAAAANAPX/FAADABcAAAANAPb/FAADABcAAAANAPf/FAADABcAAAANAPj/FAADABcAAAANAPn/FAADABcAAAANAPr/FAADABcAAAAOAO7/FAACABcAAAAOAO//FAACABgAAAAOAPD/FAACABgAAAAOAPH/FAACABgAAAAOAPL/FAACABgAAAAOAPP/FAACABgAAAAOAPT/FAACABgAAAAOAPX/FAACABgAAAAOAPb/FAACABgAAAAOAPf/FAACABgAAAAOAPj/FAACABgAAAAOAPn/FAACABgAAAAOAPr/FAACABgAAAAPAO7/FAAAAAAAAAAPAO//FAAAAAAAAAAPAPD/FAAAAAAAAAAPAPH/FAAAAAAAAAAPAPL/FAAAAAAAAAAPAPP/FAAAAAAAAAAPAPT/FAAAAAAAAAAPAPX/FAAAAAAAAAAPAPb/FAAAAAAAAAAPAPf/FAAAAAAAAAAPAPj/FAAAAAAAAAAPAPn/FAAAAAAAAAAPAPr/FAAAAAAAAAAQAO7/FAAAAAAAAAAQAO//FAAAAAAAAAAQAPD/FAAAAAAAAAAQAPH/FAAAAAAAAAAQAPL/FAAAAAAAAAAQAPP/FAAAAAAAAAAQAPT/FAAAAAAAAAAQAPX/FAAAAAAAAAAQAPb/FAAAAAAAAAAQAPf/FAAAAAAAAAAQAPj/FAAAAAAAAAAQAPn/FAAAAAAAAAAQAPr/FAAAAAAAAAARAO7/FAAAAAAAAAARAO//FAAAAAAAAAARAPD/FAAAAAAAAAARAPH/FAAAAAAAAAARAPL/FAAAAAAAAAARAPP/FAAAAAAAAAARAPT/FAAAAAAAAAARAPX/FAAAAAAAAAARAPb/FAAAAAAAAAARAPf/FAAAAAAAAAARAPj/FAAAAAAAAAARAPn/FAAAAAAAAAARAPr/FAAAAAAAAAASAO7/FAAAAAAAAAASAO//FAAAAAAAAAASAPD/FAAAAAAAAAASAPH/FAAAAAAAAAASAPL/FAAAAAAAAAASAPP/FAAAAAAAAAASAPT/FAAAAAAAAAASAPX/FAAAAAAAAAASAPb/FAAAAAAAAAASAPf/FAAAAAAAAAASAPj/FAAAAAAAAAASAPn/FAAAAAAAAAASAPr/FAAAAAAAAAABAPz/FAADAAQAAAABAP7/FAAAAAUAAAABAP3/FAAAAAQAAAABAPv/FAAAABkAAAAMAPv/FAABABkAAAANAPv/FAABABkAAAAOAPv/FAACABkAAAAMAPz/FAAGAAQAAAANAPz/FAAFAAUAAAAOAPz/FAAFAAQAAAAOAP3/FAACAAQAAAAOAP7/FAACAAUAAAAMAP3/FAABAAQAAAANAP3/FAABAAQAAAAMAP7/FAABAAUAAAANAP7/FAABAAUAAAAPAO3/FAAAAAAAAAAPAPv/FAAAAAAAAAAPAPz/FAAAAAAAAAAPAP3/FAAAAAAAAAAPAP7/FAAAAAAAAAAQAO3/FAAAAAAAAAAQAPv/FAAAAAAAAAAQAPz/FAAAAAAAAAAQAP3/FAAAAAAAAAAQAP7/FAAAAAAAAAARAO3/FAAAAAAAAAARAPv/FAAAAAAAAAARAPz/FAAAAAAAAAARAP3/FAAAAAAAAAARAP7/FAAAAAAAAAASAO3/FAAAAAAAAAASAPv/FAAAAAAAAAASAPz/FAAAAAAAAAASAP3/FAAAAAAAAAASAP7/FAAAAAAAAAATAO3/FAAAAAAAAAATAO7/FAAAAAAAAAATAO//FAAAAAAAAAATAPD/FAAAAAAAAAATAPH/FAAAAAAAAAATAPL/FAAAAAAAAAATAPP/FAAAAAAAAAATAPT/FAAAAAAAAAATAPX/FAAAAAAAAAATAPb/FAAAAAAAAAATAPf/FAAAAAAAAAATAPj/FAAAAAAAAAATAPn/FAAAAAAAAAATAPr/FAAAAAAAAAATAPv/FAAAAAAAAAATAPz/FAAAAAAAAAATAP3/FAAAAAAAAAATAP7/FAAAAAAAAAAUAO3/FAAAAAAAAAAUAO7/FAAAAAAAAAAUAO//FAAAAAAAAAAUAPD/FAAAAAAAAAAUAPH/FAAAAAAAAAAUAPL/FAAAAAAAAAAUAPP/FAAAAAAAAAAUAPT/FAAAAAAAAAAUAPX/FAAAAAAAAAAUAPb/FAAAAAAAAAAUAPf/FAAAAAAAAAAUAPj/FAAAAAAAAAAUAPn/FAAAAAAAAAAUAPr/FAAAAAAAAAAUAPv/FAAAAAAAAAAUAPz/FAAAAAAAAAAUAP3/FAAAAAAAAAAUAP7/FAAAAAAAAAAVAO3/FAAAAAAAAAAVAO7/FAAAAAAAAAAVAO//FAAAAAAAAAAVAPD/FAAAAAAAAAAVAPH/FAAAAAAAAAAVAPL/FAAAAAAAAAAVAPP/FAAAAAAAAAAVAPT/FAAAAAAAAAAVAPX/FAAAAAAAAAAVAPb/FAAAAAAAAAAVAPf/FAAAAAAAAAAVAPj/FAAAAAAAAAAVAPn/FAAAAAAAAAAVAPr/FAAAAAAAAAAVAPv/FAAAAAAAAAAVAPz/FAAAAAAAAAAVAP3/FAAAAAAAAAAVAP7/FAADAB0AAAAWAO3/FAAAAAAAAAAWAO7/FAAAAAAAAAAWAO//FAAAAAAAAAAWAPD/FAAAAAAAAAAWAPH/FAAAAAAAAAAWAPL/FAAAAAAAAAAWAPP/FAAAAAAAAAAWAPT/FAAAAAAAAAAWAPX/FAAAAAAAAAAWAPb/FAAAAAAAAAAWAPf/FAAAAAAAAAAWAPj/FAAAAAAAAAAWAPn/FAAAAAAAAAAWAPr/FAAAAAAAAAAWAPv/FAAAAAAAAAAWAPz/FAAAAAAAAAAWAP3/FAAAAAAAAAAWAP7/FAAGAB0AAAAXAO3/FAAAAAAAAAAXAO7/FAAAAAAAAAAXAO//FAAAAAAAAAAXAPD/FAAAAAAAAAAXAPH/FAAAAAAAAAAXAPL/FAAAAAAAAAAXAPP/FAAAAAAAAAAXAPT/FAAAAAAAAAAXAPX/FAAAAAAAAAAXAPb/FAAAAAAAAAAXAPf/FAAAAAAAAAAXAPj/FAAAAAAAAAAXAPn/FAAAAAAAAAAXAPr/FAAAAAAAAAAXAPv/FAAAAAAAAAAXAPz/FAAAAAAAAAAXAP3/FAADAB0AAAAXAP7/FAABAAAAAAAYAO3/FAAAAAAAAAAYAO7/FAAAAAAAAAAYAO//FAAAAAAAAAAYAPD/FAAAAAAAAAAYAPH/FAAAAAAAAAAYAPL/FAAAAAAAAAAYAPP/FAAAAAAAAAAYAPT/FAAAAAAAAAAYAPX/FAAAAAAAAAAYAPb/FAAAAAAAAAAYAPf/FAAAAAAAAAAYAPj/FAAAAAAAAAAYAPn/FAAAAAAAAAAYAPr/FAAAAAAAAAAYAPv/FAAAAAAAAAAYAPz/FAAAAAAAAAAYAP3/FAAEAB0AACAYAP7/FAABAAAAAAAYAP//FAACACsAAAAYAAAAFAABAAAAAAAYAAEAFAAFAB0AAAAYAAIAFAAAAAAAAAAYAAMAFAAAAAAAAAAYAAQAFAAAAAAAAAAYAAUAFAAAAAAAAAAYAAYAFAAAAAAAAAAYAAcAFAAAAAAAAAAYAAgAFAAAAAAAAAAYAAkAFAAAAAAAAAAYAAoAFAAAAAAAAAAYAAsAFAAAAAAAAAAYAAwAFAAAAAAAAAAYAA0AFAAAAAAAAAAYAA4AFAAAAAAAAAAYAA8AFAAAAAAAAAAYABAAFAAAAAAAAAAYABEAFAAAAAAAAAAYABIAFAAAAAAAAAAYABMAFAAAAAAAAAAYABQAFAAAAAAAAAAYABUAFAAAAAAAAAAYABYAFAAAAAAAAAAZAO3/FAAAAAAAAAAZAO7/FAAAAAAAAAAZAO//FAAAAAAAAAAZAPD/FAAAAAQAAAAZAPH/FAAAAAQAAAAZAPL/FAAAAAQAAAAZAPP/FAAAAAQAAAAZAPT/FAAAAAQAAAAZAPX/FAAAAAQAAAAZAPb/FAAAAAQAAAAZAPf/FAAAAAQAAAAZAPj/FAAAAAUAAAAZAPn/FAAAAAYAAAAZAPr/FAAAAAcAAAAZAPz/FAAAAAcAAAAZAP3/FAABAAAAAAAZAP7/FAABAAEAAAAZAP//FAABAAAAAAAZAAAAFAAFAB0AAAAZAAEAFAAAAAAAAAAZAAIAFAAAAAAAAAAZAAMAFAAAAAAAAAAZAAQAFAAAAAAAAAAZAAUAFAAAAAAAAAAZAAYAFAAAAAAAAAAZAAcAFAAAAAAAAAAZAAgAFAAAAAAAAAAZAAkAFAAAAAAAAAAZAAoAFAAAAAAAAAAZAAsAFAAAAAAAAAAZAAwAFAAAAAAAAAAZAA0AFAAAAAAAAAAZAA4AFAAAAAAAAAAZAA8AFAAAAAAAAAAZABAAFAAAAAAAAAAZABEAFAAAAAAAAAAZABIAFAAAAAAAAAAZABMAFAAAAAAAAAAZABQAFAAAAAAAAAAZABUAFAAAAAAAAAAZABYAFAAAAAAAAAAaAO3/FAAAAAAAAAAaAO7/FAAAAAAAAAAaAO//FAAAAAAAAAAaAPD/FAABAAQAAAAaAPH/FAABAAQAAAAaAPL/FAABAAQAAAAaAPP/FAABAAQAAAAaAPT/FAABAAQAAAAaAPX/FAABAAQAAAAaAPb/FAABAAQAAAAaAPf/FAADAAUAAAAaAPn/FAABAAYAAAAaAPr/FAABAAcAAAAaAPv/FAABAAcAAAAaAPz/FAABAAcAAAAaAP3/FAABAAAAAAAaAP7/FAABAAAAAAAaAP//FAAFAB0AAAAaAAAAFAAAAAAAAAAaAAEAFAAAAAAAAAAaAAIAFAAAAAAAAAAaAAMAFAAAAAAAAAAaAAQAFAAAAAAAAAAaAAUAFAAAAAAAAAAaAAYAFAAAAAAAAAAaAAcAFAAAAAAAAAAaAAgAFAAAAAAAAAAaAAkAFAAAAAAAAAAaAAoAFAAAAAAAAAAaAAsAFAAAAAAAAAAaAAwAFAAAAAAAAAAaAA0AFAAAAAAAAAAaAA4AFAAAAAAAAAAaAA8AFAAAAAAAAAAaABAAFAAAAAAAAAAaABEAFAAAAAAAAAAaABIAFAAAAAAAAAAaABMAFAAAAAAAAAAaABQAFAAAAAAAAAAaABUAFAAAAAAAAAAaABYAFAAAAAAAAAAbAO3/FAAAAAAAAAAbAO7/FAAAAAAAAAAbAO//FAAAAAAAAAAbAPD/FAABAAQAAAAbAPH/FAABAAQAAAAbAPL/FAABAAQAAAAbAPP/FAABAAQAAAAbAPT/FAABAAQAAAAbAPX/FAABAAQAAAAbAPb/FAABAAQAAAAbAPf/FAABAAQAAAAbAPj/FAABAAUAAAAbAPn/FAACAAYAAAAbAPr/FAABAAcAAAAbAPv/FAABAAcAAAAbAPz/FAABAAcAAAAbAP3/FAABAAAAAAAbAP7/FAAFAB0AAAAbAP//FAAAAAAAAAAbAAAAFAAAAAAAAAAbAAEAFAAAAAAAAAAbAAIAFAAAAAAAAAAbAAMAFAAAAAAAAAAbAAQAFAAAAAAAAAAbAAUAFAAAAAAAAAAbAAYAFAAAAAAAAAAbAAcAFAAAAAAAAAAbAAgAFAAAAAAAAAAbAAkAFAAAAAAAAAAbAAoAFAAAAAAAAAAbAAsAFAAAAAAAAAAbAAwAFAAAAAAAAAAbAA0AFAAAAAAAAAAbAA4AFAAAAAAAAAAbAA8AFAAAAAAAAAAbABAAFAAAAAAAAAAbABEAFAAAAAAAAAAbABIAFAAAAAAAAAAbABMAFAAAAAAAAAAbABQAFAAAAAAAAAAbABUAFAAAAAAAAAAbABYAFAAAAAAAAAAcAO3/FAAAAAAAAAAcAO7/FAAAAAAAAAAcAO//FAAAAAAAAAAcAPD/FAAGAAQAAAAcAPH/FAABAAQAAAAcAPL/FAABAAQAAAAcAPP/FAABAAQAAAAcAPT/FAADAAUAAAAcAPX/FAABAAQAAAAcAPb/FAAEAAUAAAAcAPf/FAABAAQAAAAcAPj/FAABAAUAAAAcAPn/FAAFACQAADAcAPr/FAABAAYAAAAcAPv/FAABAAcAAAAcAPz/FAABAAcAAAAcAP3/FAAFAB0AAAAcAP7/FAAAAAAAAAAcAP//FAAAAAAAAAAcAAAAFAAAAAAAAAAcAAEAFAAAAAAAAAAcAAIAFAAAAAAAAAAcAAMAFAAAAAAAAAAcAAQAFAAAAAAAAAAcAAUAFAAAAAAAAAAcAAYAFAAAAAAAAAAcAAcAFAAAAAAAAAAcAAgAFAAAAAAAAAAcAAkAFAAAAAAAAAAcAAoAFAAAAAAAAAAcAAsAFAAAAAAAAAAcAAwAFAAAAAAAAAAcAA0AFAAAAAAAAAAcAA4AFAAAAAAAAAAcAA8AFAAAAAAAAAAcABAAFAAAAAAAAAAcABEAFAAAAAAAAAAcABIAFAAAAAAAAAAcABMAFAAAAAAAAAAcABQAFAAAAAAAAAAcABUAFAAAAAAAAAAcABYAFAAAAAAAAAAdAO3/FAAAAAAAAAAdAO7/FAAAAAAAAAAdAO//FAAAAAAAAAAdAPD/FAAGAAQAAAAdAPH/FAABAAQAAAAdAPL/FAABAAQAAAAdAPP/FAADAAUAAAAdAPT/FAABAAQAAAAdAPX/FAABAAQAAAAdAPb/FAABAAQAAAAdAPf/FAABAAQAAAAdAPj/FAABAAUAAAAdAPn/FAAFACQAAFAdAPr/FAABAAYAAAAdAPv/FAABAAcAAAAdAPz/FAABAAcAAAAdAP3/FAAAAAAAAAAdAP7/FAAAAAAAAAAdAP//FAAAAAAAAAAdAAAAFAAAAAAAAAAdAAEAFAAAAAAAAAAdAAIAFAAAAAAAAAAdAAMAFAAAAAAAAAAdAAQAFAAAAAAAAAAdAAUAFAAAAAAAAAAdAAYAFAAAAAAAAAAdAAcAFAAAAAAAAAAdAAgAFAAAAAAAAAAdAAkAFAAAAAAAAAAdAAoAFAAAAAAAAAAdAAsAFAAAAAAAAAAdAAwAFAAAAAAAAAAdAA0AFAAAAAAAAAAdAA4AFAAAAAAAAAAdAA8AFAAAAAAAAAAdABAAFAAAAAAAAAAdABEAFAAAAAAAAAAdABIAFAAAAAAAAAAdABMAFAAAAAAAAAAdABQAFAAAAAAAAAAdABUAFAAAAAAAAAAdABYAFAAAAAAAAAAeAO3/FAAAAAAAAAAeAO7/FAAAAAAAAAAeAO//FAAAAAAAAAAeAPD/FAAGAAQAAAAeAPH/FAABAAQAAAAeAPL/FAABAAQAAAAeAPP/FAABAAQAAAAeAPT/FAABAAQAAAAeAPX/FAABAAQAAAAeAPb/FAABAAQAAAAeAPf/FAABAAQAAAAeAPj/FAABAAUAAAAeAPn/FAAFACQAAGAeAPr/FAABAAYAAAAeAPv/FAABAAcAAAAeAPz/FAABAAcAAAAeAP3/FAAAAAAAAAAeAP7/FAAAAAAAAAAeAP//FAAAAAAAAAAeAAAAFAAAAAAAAAAeAAEAFAAAAAAAAAAeAAIAFAAAAAAAAAAeAAMAFAAAAAAAAAAeAAQAFAAAAAAAAAAeAAUAFAAAAAAAAAAeAAYAFAAAAAAAAAAeAAcAFAAAAAAAAAAeAAgAFAAAAAAAAAAeAAkAFAAAAAAAAAAeAAoAFAAAAAAAAAAeAAsAFAAAAAAAAAAeAAwAFAAAAAAAAAAeAA0AFAAAAAAAAAAeAA4AFAAAAAAAAAAeAA8AFAAAAAAAAAAeABAAFAAAAAAAAAAeABEAFAAAAAAAAAAeABIAFAAAAAAAAAAeABMAFAAAAAAAAAAeABQAFAAAAAAAAAAeABUAFAAAAAAAAAAeABYAFAAAAAAAAAAfAO3/FAAAAAAAAAAfAO7/FAAAAAAAAAAfAO//FAAAAAAAAAAfAPD/FAABAAQAAAAfAPH/FAABAAQAAAAfAPL/FAABAAQAAAAfAPP/FAABAAQAAAAfAPT/FAABAAQAAAAfAPX/FAAEAAUAAAAfAPb/FAABAAQAAAAfAPf/FAABAAQAAAAfAPj/FAABAAUAAAAfAPn/FAAFACQAADAfAPr/FAABAAYAAAAfAPv/FAABAAcAAAAfAPz/FAABAAcAAAAfAP3/FAAAAAAAAAAfAP7/FAAAAAAAAAAfAP//FAAAAAAAAAAfAAAAFAAAAAAAAAAfAAEAFAAAAAAAAAAfAAIAFAAAAAAAAAAfAAMAFAAAAAAAAAAfAAQAFAAAAAAAAAAfAAUAFAAAAAAAAAAfAAYAFAAAAAAAAAAfAAcAFAAAAAAAAAAfAAgAFAAAAAAAAAAfAAkAFAAAAAAAAAAfAAoAFAAAAAAAAAAfAAsAFAAAAAAAAAAfAAwAFAAAAAAAAAAfAA0AFAAAAAAAAAAfAA4AFAAAAAAAAAAfAA8AFAAAAAAAAAAfABAAFAAAAAAAAAAfABEAFAAAAAAAAAAfABIAFAAAAAAAAAAfABMAFAAAAAAAAAAfABQAFAAAAAAAAAAfABUAFAAAAAAAAAAfABYAFAAAAAAAAAAgAO3/FAAAAAAAAAAgAO7/FAAAAAAAAAAgAO//FAAAAAAAAAAgAPD/FAABAAQAAAAgAPH/FAABAAQAAAAgAPL/FAABAAQAAAAgAPP/FAABAAQAAAAgAPT/FAABAAQAAAAgAPX/FAABAAQAAAAgAPb/FAAEAAUAAAAgAPf/FAABAAQAAAAgAPj/FAABAAUAAAAgAPr/FAABAAYAAAAgAPv/FAABAAcAAAAgAPz/FAABAAcAAAAgAP3/FAAAAAAAAAAgAP7/FAAAAAAAAAAgAP//FAAAAAAAAAAgAAAAFAAAAAAAAAAgAAEAFAAAAAAAAAAgAAIAFAAAAAAAAAAgAAMAFAAAAAAAAAAgAAQAFAAAAAAAAAAgAAUAFAAAAAAAAAAgAAYAFAAAAAAAAAAgAAcAFAAAAAAAAAAgAAgAFAAAAAAAAAAgAAkAFAAAAAAAAAAgAAoAFAAAAAAAAAAgAAsAFAAAAAAAAAAgAAwAFAAAAAAAAAAgAA0AFAAAAAAAAAAgAA4AFAAAAAAAAAAgAA8AFAAAAAAAAAAgABAAFAAAAAAAAAAgABEAFAAAAAAAAAAgABIAFAAAAAAAAAAgABMAFAAAAAAAAAAgABQAFAAAAAAAAAAgABUAFAAAAAAAAAAgABYAFAAAAAAAAAAhAO3/FAAAAAAAAAAhAO7/FAAAAAAAAAAhAO//FAAAAAAAAAAhAPD/FAABAAQAAAAhAPH/FAABAAQAAAAhAPL/FAABAAQAAAAhAPP/FAABAAQAAAAhAPT/FAADAAUAAAAhAPX/FAABAAQAAAAhAPb/FAABAAQAAAAhAPf/FAABAAQAAAAhAPj/FAABAAUAAAAhAPr/FAABAAYAAAAhAPv/FAABAAcAAAAhAPz/FAABAAcAAAAhAP3/FAAAAAAAAAAhAP7/FAAAAAAAAAAhAP//FAAAAAAAAAAhAAAAFAAAAAAAAAAhAAEAFAAAAAAAAAAhAAIAFAAAAAAAAAAhAAMAFAAAAAAAAAAhAAQAFAAAAAAAAAAhAAUAFAAAAAAAAAAhAAYAFAAAAAAAAAAhAAcAFAAAAAAAAAAhAAgAFAAAAAAAAAAhAAkAFAAAAAAAAAAhAAoAFAAAAAAAAAAhAAsAFAAAAAAAAAAhAAwAFAAAAAAAAAAhAA0AFAAAAAAAAAAhAA4AFAAAAAAAAAAhAA8AFAAAAAAAAAAhABAAFAAAAAAAAAAhABEAFAAAAAAAAAAhABIAFAAAAAAAAAAhABMAFAAAAAAAAAAhABQAFAAAAAAAAAAhABUAFAAAAAAAAAAhABYAFAAAAAAAAAAiAO3/FAAAAAAAAAAiAO7/FAAAAAAAAAAiAO//FAAAAAAAAAAiAPD/FAAGAAQAAAAiAPH/FAABAAQAAAAiAPL/FAABAAQAAAAiAPP/FAABAAQAAAAiAPT/FAABAAQAAAAiAPX/FAABAAQAAAAiAPb/FAABAAQAAAAiAPf/FAABAAQAAAAiAPj/FAABAAUAAAAiAPr/FAABAAYAAAAiAPv/FAABAAcAAAAiAPz/FAABAAcAAAAiAP3/FAAAAAAAAAAiAP7/FAAAAAAAAAAiAP//FAAAAAAAAAAiAAAAFAAAAAAAAAAiAAEAFAAAAAAAAAAiAAIAFAAAAAAAAAAiAAMAFAAAAAAAAAAiAAQAFAAAAAAAAAAiAAUAFAAAAAAAAAAiAAYAFAAAAAAAAAAiAAcAFAAAAAAAAAAiAAgAFAAAAAAAAAAiAAkAFAAAAAAAAAAiAAoAFAAAAAAAAAAiAAsAFAAAAAAAAAAiAAwAFAAAAAAAAAAiAA0AFAAAAAAAAAAiAA4AFAAAAAAAAAAiAA8AFAAAAAAAAAAiABAAFAAAAAAAAAAiABEAFAAAAAAAAAAiABIAFAAAAAAAAAAiABMAFAAAAAAAAAAiABQAFAAAAAAAAAAiABUAFAAAAAAAAAAiABYAFAAAAAAAAAAjAO3/FAAAAAAAAAAjAO7/FAAAAAAAAAAjAO//FAAAAAAAAAAjAPD/FAAGAAQAAAAjAPH/FAABAAQAAAAjAPL/FAABAAQAAAAjAPP/FAABAAQAAAAjAPT/FAABAAQAAAAjAPX/FAABAAQAAAAjAPb/FAAEAAUAAAAjAPf/FAABAAQAAAAjAPj/FAABAAUAAAAjAPr/FAABAAYAAAAjAPv/FAABAAcAAAAjAPz/FAABAAcAAAAjAP3/FAAAAAAAAAAjAP7/FAAAAAAAAAAjAP//FAAAAAAAAAAjAAAAFAAAAAAAAAAjAAEAFAAAAAAAAAAjAAIAFAAAAAAAAAAjAAMAFAAAAAAAAAAjAAQAFAAAAAAAAAAjAAUAFAAAAAAAAAAjAAYAFAAAAAAAAAAjAAcAFAAAAAAAAAAjAAgAFAAAAAAAAAAjAAkAFAAAAAAAAAAjAAoAFAAAAAAAAAAjAAsAFAAAAAAAAAAjAAwAFAAAAAAAAAAjAA0AFAAAAAAAAAAjAA4AFAAAAAAAAAAjAA8AFAAAAAAAAAAjABAAFAAAAAAAAAAjABEAFAAAAAAAAAAjABIAFAAAAAAAAAAjABMAFAAAAAAAAAAjABQAFAAAAAAAAAAjABUAFAAAAAAAAAAjABYAFAAAAAAAAAAkAO3/FAAAAAAAAAAkAO7/FAAAAAAAAAAkAO//FAAAAAAAAAAkAPD/FAABAAQAAAAkAPH/FAABAAQAAAAkAPL/FAABAAQAAAAkAPP/FAABAAQAAAAkAPT/FAABAAQAAAAkAPX/FAABAAQAAAAkAPb/FAABAAQAAAAkAPf/FAABAAQAAAAkAPj/FAABAAUAAAAkAPr/FAABAAYAAAAkAPv/FAABAAcAAAAkAPz/FAABAAcAAAAkAP3/FAAAAAAAAAAkAP7/FAAAAAAAAAAkAP//FAAAAAAAAAAkAAAAFAAAAAAAAAAkAAEAFAAAAAAAAAAkAAIAFAAAAAAAAAAkAAMAFAAAAAAAAAAkAAQAFAAAAAAAAAAkAAUAFAAAAAAAAAAkAAYAFAAAAAAAAAAkAAcAFAAAAAAAAAAkAAgAFAAAAAAAAAAkAAkAFAAAAAAAAAAkAAoAFAAAAAAAAAAkAAsAFAAAAAAAAAAkAAwAFAAAAAAAAAAkAA0AFAAAAAAAAAAkAA4AFAAAAAAAAAAkAA8AFAAAAAAAAAAkABAAFAAAAAAAAAAkABEAFAAAAAAAAAAkABIAFAAAAAAAAAAkABMAFAAAAAAAAAAkABQAFAAAAAAAAAAkABUAFAAAAAAAAAAkABYAFAAAAAAAAAAlAO3/FAAAAAAAAAAlAO7/FAAAAAAAAAAlAO//FAAAAAAAAAAlAPD/FAAGAAQAAAAlAPH/FAABAAQAAAAlAPL/FAABAAQAAAAlAPP/FAABAAQAAAAlAPT/FAABAAQAAAAlAPX/FAADAAUAAAAlAPb/FAABAAQAAAAlAPf/FAABAAQAAAAlAPj/FAABAAUAAAAlAPr/FAABAAYAAAAlAPv/FAABAAcAAAAlAPz/FAABAAcAAAAlAP3/FAAAAAAAAAAlAP7/FAAAAAAAAAAlAP//FAAAAAAAAAAlAAAAFAAAAAAAAAAlAAEAFAAAAAAAAAAlAAIAFAAAAAAAAAAlAAMAFAAAAAAAAAAlAAQAFAAAAAAAAAAlAAUAFAAAAAAAAAAlAAYAFAAAAAAAAAAlAAcAFAAAAAAAAAAlAAgAFAAAAAAAAAAlAAkAFAAAAAAAAAAlAAoAFAAAAAAAAAAlAAsAFAAAAAAAAAAlAAwAFAAAAAAAAAAlAA0AFAAAAAAAAAAlAA4AFAAAAAAAAAAlAA8AFAAAAAAAAAAlABAAFAAAAAAAAAAlABEAFAAAAAAAAAAlABIAFAAAAAAAAAAlABMAFAAAAAAAAAAlABQAFAAAAAAAAAAlABUAFAAAAAAAAAAlABYAFAAAAAAAAAAmAO3/FAAAAAAAAAAmAO7/FAAAAAAAAAAmAO//FAAAAAAAAAAmAPD/FAABAAQAAAAmAPH/FAABAAQAAAAmAPL/FAABAAQAAAAmAPP/FAADAAUAAAAmAPT/FAABAAQAAAAmAPX/FAABAAQAAAAmAPb/FAABAAQAAAAmAPf/FAABAAQAAAAmAPj/FAABAAUAAAAmAPr/FAABAAYAAAAmAPv/FAABAAcAAAAmAPz/FAABAAcAAAAmAP3/FAAAAAAAAAAmAP7/FAAAAAAAAAAmAP//FAAAAAAAAAAmAAAAFAAAAAAAAAAmAAEAFAAAAAAAAAAmAAIAFAAAAAAAAAAmAAMAFAAAAAAAAAAmAAQAFAAAAAAAAAAmAAUAFAAAAAAAAAAmAAYAFAAAAAAAAAAmAAcAFAAAAAAAAAAmAAgAFAAAAAAAAAAmAAkAFAAAAAAAAAAmAAoAFAAAAAAAAAAmAAsAFAAAAAAAAAAmAAwAFAAAAAAAAAAmAA0AFAAAAAAAAAAmAA4AFAAAAAAAAAAmAA8AFAAAAAAAAAAmABAAFAAAAAAAAAAmABEAFAAAAAAAAAAmABIAFAAAAAAAAAAmABMAFAAAAAAAAAAmABQAFAAAAAAAAAAmABUAFAAAAAAAAAAmABYAFAAAAAAAAADh/+7/FAAAAAAAAADh/+//FAAAAAAAAADh//D/FAAAAAAAAADh//H/FAAAAAAAAADh//L/FAAAAAAAAADh//P/FAAAAAAAAADh//T/FAAAAAAAAADh//X/FAAAAAAAAADh//b/FAAAAAAAAADh//f/FAAAAAAAAADh//j/FAAAAAAAAADh//n/FAAAAAAAAADh//r/FAAAAAAAAADh//v/FAAAAAAAAADh//z/FAAAAAAAAADh//3/FAAAAAAAAADh//7/FAAAAAAAAADh////FAAAAAAAAADh/wAAFAAAAAAAAADh/wEAFAAAAAAAAADh/wIAFAAAAAAAAADh/wMAFAAAAAAAAADh/wQAFAAAAAAAAADh/wUAFAAAAAAAAADh/wYAFAAAAAAAAADh/wcAFAAAAAAAAADh/wgAFAAAAAAAAADh/wkAFAAAAAAAAADh/woAFAAAAAAAAADh/wsAFAAAAAAAAADh/wwAFAAAAAAAAADh/w0AFAAAAAAAAADh/w4AFAAAAAAAAADh/w8AFAAAAAAAAADh/xAAFAAAAAAAAADh/xEAFAAAAAAAAADh/xIAFAAAAAAAAADh/xMAFAAAAAAAAADh/xQAFAAAAAAAAADh/xUAFAAAAAAAAADh/xYAFAAAAAAAAADi/+7/FAAAAAAAAADi/+//FAAAAAAAAADi//D/FAAAAAAAAADi//H/FAAAAAAAAADi//L/FAAAAAAAAADi//P/FAAAAAAAAADi//T/FAAAAAAAAADi//X/FAAAAAAAAADi//b/FAAAAAAAAADi//f/FAAAAAAAAADi//j/FAAAAAAAAADi//n/FAAAAAAAAADi//r/FAAAAAAAAADi//v/FAAAAAAAAADi//z/FAAAAAAAAADi//3/FAAAAAAAAADi//7/FAAAAAAAAADi////FAAAAAAAAADi/wAAFAAAAAAAAADi/wEAFAAAAAAAAADi/wIAFAAAAAAAAADi/wMAFAAAAAAAAADi/wQAFAAAAAAAAADi/wUAFAAAAAAAAADi/wYAFAAAAAAAAADi/wcAFAAAAAAAAADi/wgAFAAAAAAAAADi/wkAFAAAAAAAAADi/woAFAAAAAAAAADi/wsAFAAAAAAAAADi/wwAFAAAAAAAAADi/w0AFAAAAAAAAADi/w4AFAAAAAAAAADi/w8AFAAAAAAAAADi/xAAFAAAAAAAAADi/xEAFAAAAAAAAADi/xIAFAAAAAAAAADi/xMAFAAAAAAAAADi/xQAFAAAAAAAAADi/xUAFAAAAAAAAADi/xYAFAAAAAAAAADj/+7/FAAAAAAAAADj/+//FAAAAAAAAADj//D/FAAAAAAAAADj//H/FAAAAAAAAADj//L/FAAAAAAAAADj//P/FAAAAAAAAADj//T/FAAAAAAAAADj//X/FAAAAAAAAADj//b/FAAAAAAAAADj//f/FAAAAAAAAADj//j/FAAAAAAAAADj//n/FAAAAAAAAADj//r/FAAAAAAAAADj//v/FAAAAAAAAADj//z/FAAAAAAAAADj//3/FAAAAAAAAADj//7/FAAAAAAAAADj////FAAAAAAAAADj/wAAFAAAAAAAAADj/wEAFAAAAAAAAADj/wIAFAAAAAAAAADj/wMAFAAAAAAAAADj/wQAFAAAAAAAAADj/wUAFAAAAAAAAADj/wYAFAAAAAAAAADj/wcAFAAAAAAAAADj/wgAFAAAAAAAAADj/wkAFAAAAAAAAADj/woAFAAAAAAAAADj/wsAFAAAAAAAAADj/wwAFAAAAAAAAADj/w0AFAAAAAAAAADj/w4AFAAAAAAAAADj/w8AFAAAAAAAAADj/xAAFAAAAAAAAADj/xEAFAAAAAAAAADj/xIAFAAAAAAAAADj/xMAFAAAAAAAAADj/xQAFAAAAAAAAADj/xUAFAAAAAAAAADj/xYAFAAAAAAAAADk/+7/FAAAAAAAAADk/+//FAAAAAAAAADk//D/FAAAAAAAAADk//H/FAAAAAAAAADk//L/FAAAAAAAAADk//P/FAAAAAAAAADk//T/FAAAAAAAAADk//X/FAAAAAAAAADk//b/FAAAAAAAAADk//f/FAAAAAAAAADk//j/FAAAAAAAAADk//n/FAAAAAAAAADk//r/FAAAAAAAAADk//v/FAAAAAAAAADk//z/FAAAAAAAAADk//3/FAAAAAAAAADk//7/FAAAAAAAAADk////FAAAAAAAAADk/wAAFAAAAAAAAADk/wEAFAAAAAAAAADk/wIAFAAAAAAAAADk/wMAFAAAAAAAAADk/wQAFAAAAAAAAADk/wUAFAAAAAAAAADk/wYAFAAAAAAAAADk/wcAFAAAAAAAAADk/wgAFAAAAAAAAADk/wkAFAAAAAAAAADk/woAFAAAAAAAAADk/wsAFAAAAAAAAADk/wwAFAAAAAAAAADk/w0AFAAAAAAAAADk/w4AFAAAAAAAAADk/w8AFAAAAAAAAADk/xAAFAAAAAAAAADk/xEAFAAAAAAAAADk/xIAFAAAAAAAAADk/xMAFAAAAAAAAADk/xQAFAAAAAAAAADk/xUAFAAAAAAAAADk/xYAFAAAAAAAAADl/+7/FAAAAAAAAADl/+//FAAAAAAAAADl//D/FAAAAAAAAADl//H/FAAAAAAAAADl//L/FAAAAAAAAADl//P/FAAAAAAAAADl//T/FAAAAAAAAADl//X/FAAAAAAAAADl//b/FAAAAAAAAADl//f/FAAAAAAAAADl//j/FAAAAAAAAADl//n/FAAAAAAAAADl//r/FAAAAAAAAADl//v/FAAAAAAAAADl//z/FAAAAAAAAADl//3/FAAAAAAAAADl//7/FAAAAAAAAADl////FAAAAAAAAADl/wAAFAAAAAAAAADl/wEAFAAAAAAAAADl/wIAFAAAAAAAAADl/wMAFAAAAAAAAADl/wQAFAAAAAAAAADl/wUAFAAAAAAAAADl/wYAFAAAAAAAAADl/wcAFAAAAAAAAADl/wgAFAAAAAAAAADl/wkAFAAAAAAAAADl/woAFAAAAAAAAADl/wsAFAAAAAAAAADl/wwAFAAAAAAAAADl/w0AFAAAAAAAAADl/w4AFAAAAAAAAADl/w8AFAAAAAAAAADl/xAAFAAAAAAAAADl/xEAFAAAAAAAAADl/xIAFAAAAAAAAADl/xMAFAAAAAAAAADl/xQAFAAAAAAAAADl/xUAFAAAAAAAAADl/xYAFAAAAAAAAADm/+7/FAAAAAAAAADm/+//FAAAAAAAAADm//D/FAAAAAAAAADm//H/FAAAAAAAAADm//L/FAAAAAAAAADm//P/FAAAAAAAAADm//T/FAAAAAAAAADm//X/FAAAAAAAAADm//b/FAAAAAAAAADm//f/FAAAAAAAAADm//j/FAAAAAAAAADm//n/FAAAAAAAAADm//r/FAAAAAAAAADm//v/FAAAAAAAAADm//z/FAAAAAAAAADm//3/FAAAAAAAAADm//7/FAAAAAAAAADm////FAAAAAAAAADm/wAAFAAAAAAAAADm/wEAFAAAAAAAAADm/wIAFAAAAAAAAADm/wMAFAAAAAAAAADm/wQAFAAAAAAAAADm/wUAFAAAAAAAAADm/wYAFAAAAAAAAADm/wcAFAAAAAAAAADm/wgAFAAAAAAAAADm/wkAFAAAAAAAAADm/woAFAAAAAAAAADm/wsAFAAAAAAAAADm/wwAFAAAAAAAAADm/w0AFAAAAAAAAADm/w4AFAAAAAAAAADm/w8AFAAAAAAAAADm/xAAFAAAAAAAAADm/xEAFAAAAAAAAADm/xIAFAAAAAAAAADm/xMAFAAAAAAAAADm/xQAFAAAAAAAAADm/xUAFAAAAAAAAADm/xYAFAAAAAAAAADn/+7/FAAAAAAAAADn/+//FAAAAAAAAADn//D/FAAAAAAAAADn//H/FAAAAAAAAADn//L/FAAAAAAAAADn//P/FAAAAAAAAADn//T/FAAAAAAAAADn//X/FAAAAAAAAADn//b/FAAAAAAAAADn//f/FAAAAAAAAADn//j/FAAAAAAAAADn//n/FAAAAAAAAADn//r/FAAAAAAAAADn//v/FAAAAAAAAADn//z/FAAAAAAAAADn//3/FAAAAAAAAADn//7/FAAAAAAAAADn////FAAAAAAAAADn/wAAFAAAAAAAAADn/wEAFAAAAAAAAADn/wIAFAAAAAAAAADn/wMAFAAAAAAAAADn/wQAFAAAAAAAAADn/wUAFAAAAAAAAADn/wYAFAAAAAAAAADn/wcAFAAAAAAAAADn/wgAFAAAAAAAAADn/wkAFAAAAAAAAADn/woAFAAAAAAAAADn/wsAFAAAAAAAAADn/wwAFAAAAAAAAADn/w0AFAAAAAAAAADn/w4AFAAAAAAAAADn/w8AFAAAAAAAAADn/xAAFAAAAAAAAADn/xEAFAAAAAAAAADn/xIAFAAAAAAAAADn/xMAFAAAAAAAAADn/xQAFAAAAAAAAADn/xUAFAAAAAAAAADn/xYAFAAAAAAAAADo/+7/FAAAAAAAAADo/+//FAAAAAAAAADo//D/FAAAAAAAAADo//H/FAAAAAAAAADo//L/FAAAAAAAAADo//P/FAAAAAAAAADo//T/FAAAAAAAAADo//X/FAAAAAAAAADo//b/FAAAAAAAAADo//f/FAAAAAAAAADo//j/FAAAAAAAAADo//n/FAAAAAAAAADo//r/FAAAAAAAAADo//v/FAAAAAAAAADo//z/FAAAAAAAAADo//3/FAAAAAAAAADo//7/FAAAAAAAAADo////FAAAAAAAAADo/wAAFAAAAAAAAADo/wEAFAAAAAAAAADo/wIAFAAAAAAAAADo/wMAFAAAAAAAAADo/wQAFAAAAAAAAADo/wUAFAAAAAAAAADo/wYAFAAAAAAAAADo/wcAFAAAAAAAAADo/wgAFAAAAAAAAADo/wkAFAAAAAAAAADo/woAFAAAAAAAAADo/wsAFAAAAAAAAADo/wwAFAAAAAAAAADo/w0AFAAAAAAAAADo/w4AFAAAAAAAAADo/w8AFAAAAAAAAADo/xAAFAAAAAAAAADo/xEAFAAAAAAAAADo/xIAFAAAAAAAAADo/xMAFAAAAAAAAADo/xQAFAAAAAAAAADo/xUAFAAAAAAAAADo/xYAFAAAAAAAAADp/+7/FAAAAAAAAADp/+//FAAAAAAAAADp//D/FAAAAAAAAADp//H/FAAAAAAAAADp//L/FAAAAAAAAADp//P/FAAAAAAAAADp//T/FAAAAAAAAADp//X/FAAAAAAAAADp//b/FAAAAAAAAADp//f/FAAAAAAAAADp//j/FAAAAAAAAADp//n/FAAAAAAAAADp//r/FAAAAAAAAADp//v/FAAAAAAAAADp//z/FAAAAAAAAADp//3/FAAAAAAAAADp//7/FAAAAAAAAADp////FAAAAAAAAADp/wAAFAAAAAAAAADp/wEAFAAAAAAAAADp/wIAFAAAAAAAAADp/wMAFAAAAAAAAADp/wQAFAAAAAAAAADp/wUAFAAAAAAAAADp/wYAFAAAAAAAAADp/wcAFAAAAAAAAADp/wgAFAAAAAAAAADp/wkAFAAAAAAAAADp/woAFAAAAAAAAADp/wsAFAAAAAAAAADp/wwAFAAAAAAAAADp/w0AFAAAAAAAAADp/w4AFAAAAAAAAADp/w8AFAAAAAAAAADp/xAAFAAAAAAAAADp/xEAFAAAAAAAAADp/xIAFAAAAAAAAADp/xMAFAAAAAAAAADp/xQAFAAAAAAAAADp/xUAFAAAAAAAAADp/xYAFAAAAAAAAADq/+7/FAAAAAAAAADq/+//FAAAAAAAAADq//D/FAAAAAAAAADq//H/FAAAAAAAAADq//L/FAAAAAAAAADq//P/FAAAAAAAAADq//T/FAAAAAAAAADq//X/FAAAAAAAAADq//b/FAAAAAAAAADq//f/FAAAAAAAAADq//j/FAAAAAAAAADq//n/FAAAAAAAAADq//r/FAAAAAAAAADq//v/FAAAAAAAAADq//z/FAAAAAAAAADq//3/FAAAAAAAAADq//7/FAAAAAAAAADq////FAAAAAAAAADq/wAAFAAAAAAAAADq/wEAFAAAAAAAAADq/wIAFAAAAAAAAADq/wMAFAAAAAAAAADq/wQAFAAAAAAAAADq/wUAFAAAAAAAAADq/wYAFAAAAAAAAADq/wcAFAAAAAAAAADq/wgAFAAAAAAAAADq/wkAFAAAAAAAAADq/woAFAAAAAAAAADq/wsAFAAAAAAAAADq/wwAFAAAAAAAAADq/w0AFAAAAAAAAADq/w4AFAAAAAAAAADq/w8AFAAAAAAAAADq/xAAFAAAAAAAAADq/xEAFAAAAAAAAADq/xIAFAAAAAAAAADq/xMAFAAAAAAAAADq/xQAFAAAAAAAAADq/xUAFAAAAAAAAADq/xYAFAAAAAAAAADr/+7/FAAAAAAAAADr/+//FAAAAAAAAADr//D/FAAAAAAAAADr//H/FAAAAAAAAADr//L/FAAAAAAAAADr//P/FAAAAAAAAADr//T/FAAAAAAAAADr//X/FAAAAAAAAADr//b/FAAAAAAAAADr//f/FAAAAAAAAADr//j/FAAAAAAAAADr//n/FAAAAAAAAADr//r/FAAAAAAAAADr//v/FAAAAAAAAADr//z/FAAAAAAAAADr//3/FAAAAAAAAADr//7/FAAAAAAAAADr////FAAAAAAAAADr/wAAFAAAAAAAAADr/wEAFAAAAAAAAADr/wIAFAAAAAAAAADr/wMAFAAAAAAAAADr/wQAFAAAAAAAAADr/wUAFAAAAAAAAADr/wYAFAAAAAAAAADr/wcAFAAAAAAAAADr/wgAFAAAAAAAAADr/wkAFAAAAAAAAADr/woAFAAAAAAAAADr/wsAFAAAAAAAAADr/wwAFAAAAAAAAADr/w0AFAAAAAAAAADr/w4AFAAAAAAAAADr/w8AFAAAAAAAAADr/xAAFAAAAAAAAADr/xEAFAAAAAAAAADr/xIAFAAAAAAAAADr/xMAFAAAAAAAAADr/xQAFAAAAAAAAADr/xUAFAAAAAAAAADr/xYAFAAAAAAAAADs/+7/FAAAAAAAAADs/+//FAAAAAAAAADs//D/FAAAAAAAAADs//H/FAAAAAAAAADs//L/FAAAAAAAAADs//P/FAAAAAAAAADs//T/FAAAAAAAAADs//X/FAAAAAAAAADs//b/FAAAAAAAAADs//f/FAAAAAAAAADs//j/FAAAAAAAAADs//n/FAAAAAAAAADs//r/FAAAAAAAAADs//v/FAAAAAAAAADs//z/FAAAAAAAAADs//3/FAAAAAAAAADs//7/FAAAAAAAAADs////FAAAAAAAAADs/wAAFAAAAAAAAADs/wEAFAAAAAAAAADs/wIAFAAAAAAAAADs/wMAFAAAAAAAAADs/wQAFAAAAAAAAADs/wUAFAAAAAAAAADs/wYAFAAAAAAAAADs/wcAFAAAAAAAAADs/wgAFAAAAAAAAADs/wkAFAAAAAAAAADs/woAFAAAAAAAAADs/wsAFAAAAAAAAADs/wwAFAAAAAAAAADs/w0AFAAAAAAAAADs/w4AFAAAAAAAAADs/w8AFAAAAAAAAADs/xAAFAAAAAAAAADs/xEAFAAAAAAAAADs/xIAFAAAAAAAAADs/xMAFAAAAAAAAADs/xQAFAAAAAAAAADs/xUAFAAAAAAAAADs/xYAFAAAAAAAAADt/+7/FAAAAAAAAADt/+//FAAAAAAAAADt//D/FAAAAAAAAADt//H/FAAAAAAAAADt//L/FAAAAAAAAADt//P/FAAAAAAAAADt//T/FAAAAAAAAADt//X/FAAAAAAAAADt//b/FAAAAAAAAADt//f/FAAAAAAAAADt//j/FAAAAAAAAADt//n/FAAAAAAAAADt//r/FAAAAAAAAADt//v/FAAAAAAAAADt//z/FAAAAAAAAADt//3/FAAAAAAAAADt//7/FAAAAAAAAADt////FAAAAAAAAADt/wAAFAAAAAAAAADt/wEAFAAAAAAAAADt/wIAFAAAAAAAAADt/wMAFAAAAAAAAADt/wQAFAAAAAAAAADt/wUAFAAAAAAAAADt/wYAFAAAAAAAAADt/wcAFAAAAAAAAADt/wgAFAAAAAAAAADt/wkAFAAAAAAAAADt/woAFAAAAAAAAADt/wsAFAAAAAAAAADt/wwAFAAAAAAAAADt/w0AFAAAAAAAAADt/w4AFAAAAAAAAADt/w8AFAAAAAAAAADt/xAAFAAAAAAAAADt/xEAFAAAAAAAAADt/xIAFAAAAAAAAADt/xMAFAAAAAAAAADt/xQAFAAAAAAAAADt/xUAFAAAAAAAAADt/xYAFAAAAAAAAADu/+7/FAAAAAAAAADu/+//FAAAAAAAAADu//D/FAAAAAAAAADu//H/FAAAAAAAAADu//L/FAAAAAAAAADu//P/FAAAAAAAAADu//T/FAAAAAAAAADu//X/FAAAAAAAAADu//b/FAAAAAAAAADu//f/FAAAAAAAAADu//j/FAAAAAAAAADu//n/FAAAAAAAAADu//r/FAAAAAAAAADu//v/FAAAAAAAAADu//z/FAAAAAAAAADu//3/FAAAAAAAAADu//7/FAAAAAAAAADu////FAAAAAAAAADu/wAAFAAAAAAAAADu/wEAFAAAAAAAAADu/wIAFAAAAAAAAADu/wMAFAAAAAAAAADu/wQAFAAAAAAAAADu/wUAFAAAAAAAAADu/wYAFAAAAAAAAADu/wcAFAAAAAAAAADu/wgAFAAAAAAAAADu/wkAFAAAAAAAAADu/woAFAAAAAAAAADu/wsAFAAAAAAAAADu/wwAFAAAAAAAAADu/w0AFAAAAAAAAADu/w4AFAAAAAAAAADu/w8AFAAAAAAAAADu/xAAFAAAAAAAAADu/xEAFAAAAAAAAADu/xIAFAAAAAAAAADu/xMAFAAAAAAAAADu/xQAFAAAAAAAAADu/xUAFAAAAAAAAADu/xYAFAAAAAAAAADv/+7/FAAAAAAAAADv/+//FAAAAAAAAADv//D/FAAAAAAAAADv//H/FAAAAAAAAADv//L/FAAAAAAAAADv//P/FAAAAAAAAADv//T/FAAAAAAAAADv//X/FAAAAAAAAADv//b/FAAAAAAAAADv//f/FAAAAAAAAADv//j/FAAAAAAAAADv//n/FAAAAAAAAADv//r/FAAAAAAAAADv//v/FAAAAAAAAADv//z/FAAAAAAAAADv//3/FAAAAAAAAADv//7/FAAAAAAAAADv////FAAAAAAAAADv/wAAFAAAAAAAAADv/wEAFAAAAAAAAADv/wIAFAAAAAAAAADv/wMAFAAAAAAAAADv/wQAFAAAAAAAAADv/wUAFAAAAAAAAADv/wYAFAAAAAAAAADv/wcAFAAAAAAAAADv/wgAFAAAAAAAAADv/wkAFAAAAAAAAADv/woAFAAAAAAAAADv/wsAFAAAAAAAAADv/wwAFAAAAAAAAADv/w0AFAAAAAAAAADv/w4AFAAAAAAAAADv/w8AFAAAAAAAAADv/xAAFAAAAAAAAADv/xEAFAAAAAAAAADv/xIAFAAAAAAAAADv/xMAFAAAAAAAAADv/xQAFAAAAAAAAADv/xUAFAAAAAAAAADv/xYAFAAAAAAAAADw/+7/FAAAAAAAAADw/+//FAAAAAAAAADw//D/FAAAAAAAAADw//H/FAAAAAAAAADw//L/FAAAAAAAAADw//P/FAAAAAAAAADw//T/FAAAAAAAAADw//X/FAAAAAAAAADw//b/FAAAAAAAAADw//f/FAAAAAAAAADw//j/FAAAAAAAAADw//n/FAAAAAAAAADw//r/FAAAAAAAAADw//v/FAAAAAAAAADw//z/FAAAAAAAAADw//3/FAAAAAAAAADw//7/FAAAAAAAAADw////FAAAAAAAAADw/wAAFAAAAAAAAADw/wEAFAAAAAAAAADw/wIAFAAAAAAAAADw/wMAFAAAAAAAAADw/wQAFAAAAAAAAADw/wUAFAAAAAAAAADw/wYAFAAAAAAAAADw/wcAFAAAAAAAAADw/wgAFAAAAAAAAADw/wkAFAAAAAAAAADw/woAFAAAAAAAAADw/wsAFAAAAAAAAADw/wwAFAAAAAAAAADw/w0AFAAAAAAAAADw/w4AFAAAAAAAAADw/w8AFAAAAAAAAADw/xAAFAAAAAAAAADw/xEAFAAAAAAAAADw/xIAFAAAAAAAAADw/xMAFAAAAAAAAADw/xQAFAAAAAAAAADw/xUAFAAAAAAAAADw/xYAFAAAAAAAAADx/+7/FAAAAAAAAADx/+//FAAAAAAAAADx//D/FAAAAAAAAADx//H/FAAAAAAAAADx//L/FAAAAAAAAADx//P/FAAAAAAAAADx//T/FAAAAAAAAADx//X/FAAAAAAAAADx//b/FAAAAAAAAADx//f/FAAAAAAAAADx//j/FAAAAAAAAADx//n/FAAAAAAAAADx//r/FAAAAAAAAADx//v/FAAAAAAAAADx//z/FAAAAAAAAADx//3/FAAAAAAAAADx//7/FAAAAAAAAADx////FAAAAAAAAADx/wAAFAAAAAAAAADx/wEAFAAAAAAAAADx/wIAFAAAAAAAAADx/wMAFAAAAAAAAADx/wQAFAAAAAAAAADx/wUAFAAAAAAAAADx/wYAFAAAAAAAAADx/wcAFAAAAAAAAADx/wgAFAAAAAAAAADx/wkAFAAAAAAAAADx/woAFAAAAAAAAADx/wsAFAAAAAAAAADx/wwAFAAAAAAAAADx/w0AFAAAAAAAAADx/w4AFAAAAAAAAADx/w8AFAAAAAAAAADx/xAAFAAAAAAAAADx/xEAFAAAAAAAAADx/xIAFAAAAAAAAADx/xMAFAAAAAAAAADx/xQAFAAAAAAAAADx/xUAFAAAAAAAAADx/xYAFAAAAAAAAADy/+7/FAAAAAAAAADy/+//FAAAAAAAAADy//D/FAAAAAAAAADy//H/FAAAAAAAAADy//L/FAAAAAAAAADy//P/FAAAAAAAAADy//T/FAAAAAAAAADy//X/FAAAAAAAAADy//b/FAAAAAAAAADy//f/FAAAAAAAAADy//j/FAAAAAAAAADy//n/FAAAAAAAAADy//r/FAAAAAAAAADy//v/FAAAAAAAAADy//z/FAAAAAAAAADy//3/FAAAAAAAAADy//7/FAAAAAAAAADy////FAAAAAAAAADy/wAAFAAAAAAAAADy/wEAFAAAAAAAAADy/wIAFAAAAAAAAADy/wMAFAAAAAAAAADy/wQAFAAAAAAAAADy/wUAFAAAAAAAAADy/wYAFAAAAAAAAADy/wcAFAAAAAAAAADy/wgAFAAAAAAAAADy/wkAFAAAAAAAAADy/woAFAAAAAAAAADy/wsAFAAAAAAAAADy/wwAFAAAAAAAAADy/w0AFAAAAAAAAADy/w4AFAAAAAAAAADy/w8AFAAAAAAAAADy/xAAFAAAAAAAAADy/xEAFAAAAAAAAADy/xIAFAAAAAAAAADy/xMAFAAAAAAAAADy/xQAFAAAAAAAAADy/xUAFAAAAAAAAADy/xYAFAAAAAAAAADz/+7/FAAAAAAAAADz/+//FAAAAAAAAADz//D/FAAAAAAAAADz//H/FAAAAAAAAADz//L/FAAAAAAAAADz//P/FAAAAAAAAADz//T/FAAAAAAAAADz//X/FAAAAAAAAADz//b/FAAAAAAAAADz//f/FAAAAAAAAADz//j/FAAAAAAAAADz//n/FAAAAAAAAADz//r/FAAAAAAAAADz//v/FAAAAAAAAADz//z/FAAAAAAAAADz//3/FAAAAAAAAADz//7/FAAAAAAAAADz////FAAAAAAAAADz/wAAFAAAAAAAAADz/wEAFAAAAAAAAADz/wIAFAAAAAAAAADz/wMAFAAAAAAAAADz/wQAFAAAAAAAAADz/wUAFAAAAAAAAADz/wYAFAAAAAAAAADz/wcAFAAAAAAAAADz/wgAFAAAAAAAAADz/wkAFAAAAAAAAADz/woAFAAAAAAAAADz/wsAFAAAAAAAAADz/wwAFAAAAAAAAADz/w0AFAAAAAAAAADz/w4AFAADABoAAADz/w8AFAAAAAAAAADz/xAAFAAAAAAAAADz/xEAFAAAAAAAAADz/xIAFAAAAAAAAADz/xMAFAAAAAAAAADz/xQAFAAAAAAAAADz/xUAFAAAAAAAAADz/xYAFAAAAAAAAAD0/+7/FAAAAAAAAAD0/+//FAAAAAAAAAD0//D/FAAAAAAAAAD0//H/FAAAAAAAAAD0//L/FAAAAAAAAAD0//P/FAAAAAAAAAD0//T/FAAAAAAAAAD0//X/FAAAAAAAAAD0//b/FAAAAAAAAAD0//f/FAAAAAAAAAD0//j/FAAAAAAAAAD0//n/FAAAAAAAAAD0//r/FAAAAAAAAAD0//v/FAADABoAAAD0//z/FAADABoAAAD0//3/FAADABoAAAD0//7/FAAAAAAAAAD0////FAAAAAAAAAD0/wAAFAAAAAAAAAD0/wEAFAAAAAAAAAD0/wIAFAAAAAAAAAD0/wMAFAAAAAAAAAD0/wQAFAAAAAAAAAD0/wUAFAAAAAAAAAD0/wYAFAAAAAAAAAD0/wcAFAAAAAAAAAD0/wgAFAAAAAAAAAD0/wkAFAAAAAAAAAD0/woAFAAAAAAAAAD0/wsAFAAAAAAAAAD0/wwAFAAAAAAAAAD0/w0AFAAGAB0AAAD0/w4AFAABAAAAAAD0/xAAFAAAAAAAAAD0/xEAFAAAAAAAAAD0/xIAFAAAAAAAAAD0/xMAFAAAAAAAAAD0/xQAFAAAAAAAAAD0/xUAFAAAAAAAAAD0/xYAFAAAAAAAAAD1/+7/FAAAAAAAAAD1/+//FAAAAAAAAAD1//D/FAAAAAAAAAD1//H/FAAAAAAAAAD1//L/FAAAAAAAAAD1//P/FAAAAAAAAAD1//T/FAAAAAAAAAD1//X/FAAAAAAAAAD1//b/FAAAAAAAAAD1//f/FAAAAAAAAAD1//j/FAAAAAAAAAD1//n/FAAAAAAAAAD1//r/FAAGAB0AAAD1//v/FAABAAAAAAD1//z/FAABAAAAAAD1//3/FAABAAAAAAD1//7/FAAEAB0AAAD1////FAAAAAAAAAD1/wAAFAAAAAAAAAD1/wEAFAAAAAAAAAD1/wIAFAAAAAAAAAD1/wMAFAAAAAAAAAD1/wQAFAAAAAAAAAD1/wUAFAAAAAAAAAD1/wYAFAAAAAAAAAD1/wcAFAAAAAAAAAD1/wgAFAAAAAAAAAD1/wkAFAAAAAAAAAD1/woAFAAAAAAAAAD1/wsAFAAAAAAAAAD1/wwAFAADAB0AAAD1/w0AFAABAAAAAAD1/w4AFAACACkAAAD1/w8AFAABAAAAAAD1/xAAFAADABwAAAD1/xEAFAAAAAAAAAD1/xIAFAAAAAAAAAD1/xMAFAAAAAAAAAD1/xQAFAAAAAAAAAD1/xUAFAAAAAAAAAD1/xYAFAAAAAAAAAD2/+7/FAAAAAAAAAD2/+//FAAAAAAAAAD2//D/FAAAAAAAAAD2//H/FAAAAAAAAAD2//L/FAAAAAAAAAD2//P/FAAAAAAAAAD2//T/FAAAAAAAAAD2//X/FAAAAAAAAAD2//b/FAAAAAAAAAD2//f/FAAAAAAAAAD2//j/FAAAAAAAAAD2//n/FAAAAAAAAAD2//r/FAABAAAAAAD2//v/FAABAAAAAAD2//z/FAABAAAAAAD2//3/FAABAAAAAAD2//7/FAABAAAAAAD2////FAAEAB0AAAD2/wAAFAAAAAAAAAD2/wEAFAAAAAAAAAD2/wIAFAAAAAAAAAD2/wMAFAAAAAAAAAD2/wQAFAAAAAAAAAD2/wUAFAAAAAAAAAD2/wYAFAAAAAAAAAD2/wcAFAAAAAAAAAD2/wgAFAAAAAAAAAD2/wkAFAAAAAAAAAD2/woAFAAAAAAAAAD2/wsAFAAAAAAAAAD2/wwAFAADAB0AAAD2/w0AFAABAAAAAAD2/w4AFAABAAAAAAD2/w8AFAABAAAAAAD2/xAAFAADABwAAAD2/xEAFAAAAAAAAAD2/xIAFAAAAAAAAAD2/xMAFAAAAAAAAAD2/xQAFAAAAAAAAAD2/xUAFAAAAAAAAAD2/xYAFAAAAAAAAAD0/+3/FAAAAAAAAAD1/+3/FAAAAAAAAAD2/+3/FAAAAAAAAAD3/+3/FAAAAAAAAAD3/+7/FAAAAAAAAAD3/+//FAAAAAAAAAD3//D/FAAAAAAAAAD3//H/FAAAAAAAAAD3//L/FAAAAAAAAAD3//P/FAAAAAAAAAD3//T/FAAAAAAAAAD3//X/FAAAAAAAAAD3//b/FAAAAAAAAAD3//f/FAAAAAAAAAD3//j/FAAAAAAAAAD3//n/FAAAAAAAAAD3//r/FAABAAAAAAD3//v/FAABAAAAAAD3//z/FAABAAAAAAD3//3/FAABAAAAAAD3//7/FAABAAAAAAD4/+3/FAAAAAAAAAD4/+7/FAAAAAAAAAD4/+//FAAAAAAAAAD4//D/FAAAAAAAAAD4//H/FAAAAAAAAAD4//L/FAAAAAAAAAD4//P/FAAAAAAAAAD4//T/FAAAAAAAAAD4//X/FAAAAAAAAAD4//b/FAAAAAAAAAD4//f/FAAAAAAAAAD4//j/FAAAAAAAAAD4//n/FAAAAAAAAAD4//r/FAABAAAAAAD4//v/FAABAAAAAAD4//z/FAABAAAAAAD4//3/FAABAAAAAAD4//7/FAABAAAAAAD5/+3/FAAAAAAAAAD5/+7/FAAAAAAAAAD5/+//FAAAAAAAAAD5//D/FAAAAAAAAAD5//H/FAAAAAAAAAD5//L/FAAAAAAAAAD5//P/FAAAAAAAAAD5//T/FAAAAAAAAAD5//X/FAAAAAAAAAD5//b/FAAAAAAAAAD5//f/FAAAAAAAAAD5//j/FAAAAAAAAAD5//n/FAAAAAAAAAD5//r/FAAHAB0AAAD5//v/FAABAAAAAAD5//z/FAABAAAAAAD5//3/FAABAAAAAAD5//7/FAABAAAAAAD6/+3/FAAAAAAAAAD6/+7/FAAAAAAAAAD6/+//FAAAAAAAAAD6//D/FAAAAAAAAAD6//H/FAAAAAAAAAD6//L/FAAAAAAAAAD6//P/FAAAAAAAAAD6//T/FAAAAAAAAAD6//X/FAAAAAAAAAD6//b/FAAAAAAAAAD6//f/FAAAAAAAAAD6//j/FAAAAAAAAAD6//n/FAAAAAAAAAD6//r/FAAAAAAAAAD6//v/FAADABsAAAD6//z/FAADABsAAAD6//3/FAADABsAAAD6//7/FAAHAB0AAAD7/+3/FAAAAAAAAAD7/+7/FAAAAAAAAAD7/+//FAAAAAAAAAD7//D/FAAAAAAAAAD7//H/FAAAAAAAAAD7//L/FAAAAAAAAAD7//P/FAAAAAAAAAD7//T/FAAAAAAAAAD7//X/FAAAAAAAAAD7//b/FAAAAAAAAAD7//f/FAAAAAAAAAD7//j/FAAAAAAAAAD7//n/FAAAAAAAAAD7//r/FAAAAAAAAAD7//v/FAAAAAAAAAD7//z/FAAAAAAAAAD7//3/FAAAAAAAAAD7//7/FAADAB0AAAD8/+3/FAAAAAAAAAD8//v/FAAAAAAAAAD8//z/FAAAAAAAAAD8//3/FAAAAAAAAAD8//7/FAAAAAAAAAD9/+3/FAAAAAAAAAD9//v/FAAAAAAAAAD9//z/FAAAAAAAAAD9//3/FAAAAAAAAAD9//7/FAAAAAAAAAD+/+3/FAAAAAAAAAD+//v/FAAAAAAAAAD+//z/FAAAAAAAAAD+//3/FAAAAAAAAAD+//7/FAAAAAAAAAD//+3/FAAAAAAAAAD///v/FAAAAAAAAAD///z/FAAAAAAAAAD///3/FAAAAAAAAAD///7/FAAAAAAAAAAAAO3/FAAAAAAAAAAAAPv/FAAAAAAAAAAAAPz/FAAAAAAAAAAAAP3/FAAAAAAAAAAAAP7/FAAAAAAAAAD4////FAABAAAAAAAWAAMAFAAFAB0AAAD0/w8AFAAEAB0AAAAZAPv/FAAAAAcAAAAnAPz/FAABAAcAAAAoAPz/FAABAAcAAAApAPz/FAABAAcAAAAqAPz/FAABAAcAAAAnAPr/FAABAAYAAAAnAPv/FAABAAcAAAAoAPr/FAABAAYAAAAoAPv/FAABAAcAAAApAPr/FAABAAYAAAApAPv/FAABAAcAAAAqAPr/FAABAAYAAAAqAPv/FAABAAcAAAArAPr/FAABAAYAAAArAPv/FAACAAcAAAArAPz/FAACAAcAAAAaAPj/FAABAAUAAAAnAPj/FAABAAUAAAAoAPj/FAABAAUAAAApAPj/FAABAAUAAAAqAPj/FAABAAUAAAArAPj/FAACAAUAAAAnAPD/FAAGAAQAAAAnAPH/FAABAAQAAAAnAPL/FAABAAQAAAAnAPP/FAABAAQAAAAnAPT/FAABAAQAAAAnAPX/FAABAAQAAAAnAPb/FAADAAUAAAAnAPf/FAABAAQAAAAoAPD/FAAGAAQAAAAoAPH/FAABAAQAAAAoAPL/FAABAAQAAAAoAPP/FAABAAQAAAAoAPT/FAABAAQAAAAoAPX/FAABAAQAAAAoAPb/FAABAAQAAAAoAPf/FAADAAUAAAApAPD/FAAGAAQAAAApAPH/FAABAAQAAAApAPL/FAABAAQAAAApAPP/FAABAAQAAAApAPT/FAABAAQAAAApAPX/FAABAAQAAAApAPb/FAABAAQAAAApAPf/FAABAAQAAAAqAPD/FAABAAQAAAAqAPH/FAABAAQAAAAqAPL/FAABAAQAAAAqAPP/FAABAAQAAAAqAPT/FAABAAQAAAAqAPX/FAABAAQAAAAqAPb/FAABAAQAAAAqAPf/FAABAAQAAAArAPD/FAACAAQAAAArAPH/FAACAAQAAAArAPL/FAACAAQAAAArAPP/FAACAAQAAAArAPT/FAACAAQAAAArAPX/FAACAAQAAAArAPb/FAACAAQAAAArAPf/FAACAAQAAAAgAPn/FAAFACQAAFAhAPn/FAAFACQAAAAiAPn/FAAFACQAAFAjAPn/FAAFACQAAGAkAPn/FAAFACQAAFAlAPn/FAAFACQAAAAmAPn/FAAFACQAAFAnAPn/FAAFACQAAGAoAPn/FAAFACQAAAApAPn/FAAFACQAAFAqAPn/FAAFACQAAGArAPn/FAAFACQAAAAXABMAFAAAAAAAAAA=")
tile_set = ExtResource("1_8drhf")

[node name="TileMapLayer2" type="TileMapLayer" parent="TileMapLayers" unique_id=511923808]
visible = false
z_index = -1
tile_map_data = PackedByteArray("AAAFAP7/FAAGACkAAAAGAP7/FAAGACkAAAAHAP7/FAAGACkAAAAIAP7/FAAGACkAAAAFAP3/FAAGACgAAAAGAP3/FAAGACgAAAAHAP3/FAAGACgAAAAIAP3/FAAGACgAAAAFAPz/FAAGACgAAAAGAPz/FAAGACgAAAAHAPz/FAAGACgAAAAIAPz/FAAGACgAAAA=")
tile_set = ExtResource("1_8drhf")

[node name="TextureRect" type="ColorRect" parent="." unique_id=893972823]
z_index = 4
material = SubResource("ShaderMaterial_famss")
offset_left = -2158.0
offset_top = -1612.0
offset_right = 3024.0
offset_bottom = 1729.0

[node name="Cinematic" type="Node2D" parent="." unique_id=1491722087]
script = ExtResource("3_a1j0i")
dialogue = ExtResource("4_ilpog")
metadata/_custom_type_script = "uid://x1mxt6bmei2o"

[node name="Charlie" parent="." unique_id=1958946414 instance=ExtResource("5_8cyi3")]
position = Vector2(14, -128)
scale = Vector2(0.75, 0.75)
dialogue = ExtResource("6_8cyi3")
npc_name = "Charlie"
look_at_side = 1
sprite_frames = ExtResource("7_321x3")

[node name="CollisionShape2D2" type="CollisionShape2D" parent="Charlie" unique_id=1061988001]
position = Vector2(-1.3333321, -25.333328)
shape = SubResource("RectangleShape2D_8cyi3")

[node name="Player" parent="." unique_id=894731746 instance=ExtResource("8_ukjsk")]
y_sort_enabled = true
position = Vector2(435.00003, 347)
scale = Vector2(0.75, 0.75)
sprite_frames = ExtResource("9_cbqvq")

[node name="PlayerSprite" parent="Player" index="2" unique_id=1785485617]
position = Vector2(-2.6667075, -73.333336)
sprite_frames = ExtResource("9_cbqvq")

[node name="CollisionShape2D" parent="Player/PlayerInteraction/InteractZone" parent_id_path=PackedInt32Array(894731746, 888605377) index="0" unique_id=255765935]
position = Vector2(53.333294, -73.333336)

[node name="Camera2D" type="Camera2D" parent="Player" unique_id=1018623546]
process_mode = 3
zoom = Vector2(0.902, 0.902)
limit_left = -1606
limit_top = -955
limit_right = 2426
limit_bottom = 1339
position_smoothing_enabled = true
editor_draw_limits = true

[node name="CollisionShape2D2" type="CollisionShape2D" parent="Player" unique_id=704138138]
position = Vector2(1.333292, -24)
shape = SubResource("CapsuleShape2D_p26tb")

[node name="ScreenOverlay" type="CanvasLayer" parent="." unique_id=1029527750]

[node name="HUD" parent="." unique_id=1954513484 instance=ExtResource("10_8cyi3")]

[node name="CollectibleItem" parent="." unique_id=1474014550 instance=ExtResource("11_gek8f")]
modulate = Color(1, 1, 1, 0)
position = Vector2(452, -41)
revealed = false
next_scene = "uid://iywmch2pilxk"
item = SubResource("Resource_cbqvq")
collected_dialogue = ExtResource("13_ea4l7")

[node name="SequencePuzzle" type="Node2D" parent="." unique_id=489150957]
script = ExtResource("14_0n12u")
metadata/_custom_type_script = "uid://c68oh8dtr21ti"

[node name="Objects" type="Node2D" parent="SequencePuzzle" unique_id=1640987615]
y_sort_enabled = true
position = Vector2(711, 62)
scale = Vector2(1.47, 1.47)

[node name="Blue" parent="SequencePuzzle/Objects" unique_id=1820942681 instance=ExtResource("15_ea4l7")]
sprite_frames = ExtResource("16_8drhf")
audio_stream = ExtResource("17_2yj5h")

[node name="Pink" parent="SequencePuzzle/Objects" unique_id=500515709 instance=ExtResource("15_ea4l7")]
position = Vector2(54.421745, 9.536743e-07)
sprite_frames = ExtResource("18_ls33p")
audio_stream = ExtResource("17_2yj5h")

[node name="Yellow" parent="SequencePuzzle/Objects" unique_id=1356411320 instance=ExtResource("15_ea4l7")]
position = Vector2(108.16326, -9.536743e-07)
sprite_frames = ExtResource("19_fikee")
audio_stream = ExtResource("17_2yj5h")

[node name="Green" parent="SequencePuzzle/Objects" unique_id=1167715499 instance=ExtResource("15_ea4l7")]
position = Vector2(162.585, -3.8146973e-06)
sprite_frames = ExtResource("20_g31nb")
audio_stream = ExtResource("17_2yj5h")

[node name="Steps" type="Node2D" parent="SequencePuzzle" unique_id=1475626406]

[node name="SequencePuzzleStep1" type="Node2D" parent="SequencePuzzle/Steps" unique_id=334204405 node_paths=PackedStringArray("sequence", "hint_sign")]
script = ExtResource("21_up7bl")
sequence = [NodePath("../../Objects/Yellow"), NodePath("../../Objects/Blue"), NodePath("../../Objects/Pink"), NodePath("../../Objects/Green")]
hint_sign = NodePath("../../Signs/HintSign1")

[node name="Signs" type="Node2D" parent="SequencePuzzle" unique_id=1977189161]
y_sort_enabled = true

[node name="HintSign1" parent="SequencePuzzle/Signs" unique_id=586915907 instance=ExtResource("22_0n12u")]
position = Vector2(2856, -1392)

[node name="RevealTilemap" type="Node" parent="." unique_id=1751913566 node_paths=PackedStringArray("puzzle", "tilemap_layer")]
script = ExtResource("23_ea4l7")
puzzle = NodePath("../SequencePuzzle")
tilemap_layer = NodePath("../TileMapLayers/TileMapLayer2")

[node name="AudioStreamPlayer" type="AudioStreamPlayer" parent="." unique_id=792835331]
stream = ExtResource("24_8drhf")
volume_db = 4.837
autoplay = true
bus = &"Music"
script = ExtResource("25_ls33p")

[node name="TileMapLayer" type="TileMapLayer" parent="." unique_id=2110659071]
y_sort_enabled = true
tile_map_data = PackedByteArray("AAAEAP//FAAEAAcAAAAFAP//FAAFAAcAAAAGAP//FAAFAAcAAAAHAP//FAAFAAcAAAAIAP//FAAFAAcAAAAJAP//FAAGAAcAAAANAPn/FAAHABMAAAAGAPj/FAAHABMAAAANAPH/FAAHABMAABANAPX/FAAHABMAAAAJAAAAEwAGAAsAAGAKAP//FAAEAAEAAAD1/wwAFAABAA0AAAADAP7/FAAFAAIAAAAAAAcAFAAFAAAAAAABAAgAFAAHAAAAAAAAAAYAFAAHAAEAAAABAAcAFAAGAAYAAFD3/wwAFAAAAA8AAAD7/wwAFAAAAA8AAAD//wwAFAAAAA8AAAD5/wwAFAABAA0AAAD9/wwAFAABAA0AAAABAAwAFAABAA0AAAD1/w4AFAAGAAIAAAD3//r/FAAEAA4AAAD6//n/FAAAAAwAAAD0//n/FAAAAAwAAAD7//z/FAAAAAwAAAD2////FAAAAAwAAAD9/wcAFAACAA8AAAD4/wUAFAACAA8AAAD0/wkAFAACAA8AAAD0/wUAFAAAAAwAAAD5//z/FAAHACkAAAD2/w0AFAAGACoAAAD0/w0AFAAGACsAAAAPAPv/FAACACcAAAAPAPz/FAACACgAAAAQAPv/FAACACcAAAAQAPz/FAACACgAAAARAPv/FAADACcAAAARAPz/FAADACgAAAASAPv/FAACACcAAAASAPz/FAACACgAAAATAPv/FAADACcAAAATAPz/FAADACgAAAAUAPv/FAACACcAAAAUAPz/FAACACgAAAAVAPv/FAADACcAAAAVAPz/FAADACgAAAAWAPz/FAACACgAAAAXAPv/FAADACcAAAAXAPz/FAADACgAAAAYAPz/FAACACgAAAAaAPH/FAABABMAAAAdAPH/FAABABMAAAAhAPH/FAABABMAAAAkAPH/FAABABMAAAApAPH/FAABABMAAAAbAPX/FAAEABUAAAAhAPX/FAAEABUAAAAnAPX/FAAEABUAAAAqAPX/FAAEABUAAAAkAPX/FAAAABMAAAAeAPX/FAAAABMAAAApAPX/FAAAABMAAAAaAPX/FAACABMAAAAcAPr/FAACAAAAAAAgAPz/FAAFAAYAAAAgAPv/FAADAAAAAAAdAPv/FAADAAAAAAAgAPr/FAADAAAAAAAhAPr/FAADAAAAAAAlAPz/FAAFAAYAAAAZAPn/FAACAAAAAAAdAPz/FAAHACoAAAAdAPb/FAAFACkAAAAgAPb/FAAFACkAAAAjAPb/FAAFACkAAAAmAPb/FAAFACkAAAApAPb/FAAFACkAAAAeAPz/FAAFAAYAAAAfAPz/FAAFAAYAAAAhAPz/FAAFAAYAAAAiAPz/FAAFAAYAAAAjAPz/FAAFAAYAAAAkAPz/FAAFAAYAAAAmAPz/FAAFAAYAAAAhAPj/FAAAAAIAAAAjAPj/FAAAAAIAAAAlAPj/FAAAAAIAAAAnAPj/FAAAAAIAAAApAPj/FAAAAAIAAAArAPj/FAAAAAIAAAAkAPn/FAADAAEAAAAQAPr/FAADAAIAAAATAPj/FAADAAIAAAASAPn/FAAEAAEAAAAWAPv/FAACACcAAAAVAPr/FAAEAAEAAAAXAPn/FAAFAAAAAAARAPn/FAABAAIAAAAYAPj/FAAHAAkAAAAYAPr/FAAFAAoAAAALAAsAFAADAB4AAAAMAAkAFAAAAB4AAAANAAYAFAAAAB4AAAANAAcAFAADAB4AAAAOAAYAFAADAB4AAAAOAAcAFAADAB4AAAAQAAYAFAAAAB4AAAAQAAcAFAADAB4AAAARAAkAFAADAB4AAAARAAoAFAADAB4AAAALAAoAFAADAB4AAAAMAAoAFAAAAB4AAAAMAAUAFAADAB4AAAAMAAYAFAAAAB4AAAAOAAoAFAADAB4AAAAOAAsAFAADAB4AAAAPAAoAFAAAAB4AAAAPAAsAFAADAB4AAAAQAAoAFAAAAB4AAAAQAAsAFAADAB4AAAARAAUAFAAAAB4AAAARAAYAFAAAAB4AAAASAAUAFAAAAB4AAAASAAYAFAAAAB4AAAAUAAYAFAAAAB4AAAAUAAcAFAADAB4AAAAVAAYAFAAAAB4AAAAVAAcAFAADAB4AAAALAAgAFAAAAB4AAAAPAAkAFAADAB4AAAAQAAkAFAAAAB4AAAAMAAcAFAADAB4AAAAMAAgAFAAAAB4AAAAPAAgAFAAAAB4AAAAQAAgAFAADAB4AAAARAAgAFAADAB4AAAASAAgAFAAAAB4AAAASAAkAFAADAB4AAAAUAAkAFAADAB4AAAATAAkAFAAAAB4AAAATAAoAFAAAAB4AAAATAAsAFAADAB4AAAAUAAoAFAADAB4AAAAUAAsAFAAAAB4AAAAVAAkAFAAAAB4AAAAVAAoAFAADAB4AAAAVAAsAFAAAAB4AAAANAAgAFAAAAB4AAAAOAAgAFAAAAB4AAAAPAAYAFAAAAB4AAAAPAAcAFAADAB4AAAARAAcAFAADAB4AAAASAAcAFAAAAB4AAAATAAYAFAADAB4AAAATAAcAFAAAAB4AAAAMAAsAFAAAAB4AAAANAAsAFAAAAB4AAAAWAAsAFAADAB4AAAAXAAsAFAAAAB4AAAAWAAUAFAAAAB4AAAAWAAYAFAAAAB4AAAAWAAcAFAAAAB4AAAAWAAgAFAAAAB4AAAAWAAkAFAADAB4AAAAWAAoAFAADAB4AAAAXAAUAFAAAAB4AAAAXAAYAFAADAB4AAAAXAAcAFAADAB4AAAAXAAgAFAADAB4AAAAXAAkAFAAAAB4AAAAXAAoAFAADAB4AAAAYAAUAFAADAB4AAAAYAAYAFAADAB4AAAAYAAcAFAAAAB4AAAAYAAgAFAADAB4AAAAYAAkAFAADAB4AAAAYAAoAFAAAAB4AAAAYAAsAFAADAB4AAAAZAAUAFAAAAB4AAAAZAAYAFAADAB4AAAAZAAcAFAAAAB4AAAAZAAgAFAADAB4AAAAZAAkAFAAAAB4AAAAZAAoAFAADAB4AAAAZAAsAFAADAB4AAAAaAAUAFAAAAB4AAAAaAAYAFAAAAB4AAAAaAAcAFAADAB4AAAAaAAgAFAAAAB4AAAAaAAkAFAAAAB4AAAAaAAoAFAAAAB4AAAAaAAsAFAADAB4AAAAbAAUAFAAAAB4AAAAbAAYAFAADAB4AAAAbAAcAFAADAB4AAAAbAAgAFAADAB4AAAAbAAkAFAADAB4AAAAbAAoAFAAAAB4AAAAbAAsAFAAAAB4AAAAcAAUAFAADAB4AAAAcAAYAFAAAAB4AAAAcAAcAFAAAAB4AAAAcAAgAFAAAAB4AAAAcAAkAFAADAB4AAAAcAAoAFAADAB4AAAAcAAsAFAADAB4AAAAdAAUAFAAAAB4AAAAdAAYAFAADAB4AAAAdAAcAFAADAB4AAAAdAAgAFAADAB4AAAAdAAkAFAAAAB4AAAAdAAoAFAADAB4AAAAdAAsAFAADAB4AAAAeAAUAFAAAAB4AAAAeAAYAFAAAAB4AAAAeAAcAFAAAAB4AAAAeAAgAFAAAAB4AAAAeAAkAFAADAB4AAAAeAAoAFAADAB4AAAAeAAsAFAAAAB4AAAAfAAUAFAAAAB4AAAAfAAYAFAAAAB4AAAAfAAcAFAAAAB4AAAAfAAgAFAAAAB4AAAAfAAkAFAADAB4AAAAfAAoAFAADAB4AAAAfAAsAFAADAB4AAAAgAAUAFAADAB4AAAAgAAYAFAAAAB4AAAAgAAcAFAAAAB4AAAAgAAgAFAADAB4AAAAgAAkAFAAAAB4AAAAgAAoAFAADAB4AAAAgAAsAFAAAAB4AAAAhAAUAFAAAAB4AAAAhAAYAFAAAAB4AAAAhAAcAFAAAAB4AAAAhAAgAFAADAB4AAAAhAAkAFAADAB4AAAAhAAoAFAAAAB4AAAAhAAsAFAAAAB4AAAAiAAUAFAADAB4AAAAiAAYAFAAAAB4AAAAiAAcAFAAAAB4AAAAiAAgAFAAAAB4AAAAiAAkAFAAAAB4AAAAiAAoAFAADAB4AAAAiAAsAFAAAAB4AAAAjAAUAFAADAB4AAAAjAAYAFAADAB4AAAAjAAcAFAADAB4AAAAjAAgAFAADAB4AAAAjAAkAFAAAAB4AAAAjAAoAFAAAAB4AAAAjAAsAFAADAB4AAAAkAAUAFAADAB4AAAAkAAYAFAAAAB4AAAAkAAcAFAADAB4AAAAkAAgAFAAAAB4AAAAkAAkAFAADAB4AAAAkAAoAFAADAB4AAAAkAAsAFAADAB4AAAAlAAUAFAAAAB4AAAAlAAYAFAADAB4AAAAlAAcAFAADAB4AAAAlAAgAFAAAAB4AAAAlAAkAFAAAAB4AAAAlAAoAFAAAAB4AAAAlAAsAFAAAAB4AAAAmAAUAFAADAB4AAAAmAAYAFAADAB4AAAAmAAcAFAADAB4AAAAmAAgAFAADAB4AAAAmAAkAFAAAAB4AAAAmAAoAFAAAAB4AAAAmAAsAFAADAB4AAAARABAAFAAHACAAAAARABEAFAAGACAAAAARABIAFAAFACEAAAASABAAFAAHACAAAAASABEAFAAGACAAAAASABIAFAAGACEAAAATABAAFAAHACAAAAATABEAFAAHACEAAAATABIAFAAGACIAAAAUABAAFAAGACAAAAAUABEAFAAHACEAAAAUABIAFAAHACIAAAAVABAAFAAGACIAAAAVABEAFAAHACIAAAAVABIAFAAHACAAAAD1//v/FAAFAAkAAAD5//v/FAAHAAYAAADz//z/FAABAAIAAAD8//n/FAABAAIAAADz////FAACAAEAAADz//v/FAACAAEAAAD7//r/FAACAAEAAAD9//z/FAACAAEAAAD3/wkAFAACAAEAAAD7/wQAFAACAAEAAAD2/wMAFAACAAEAAAD8/woAFAACAAEAAAACAAYAFAACAAEAAAACAAoAFAACAAEAAADy/woAFAACAAEAAADx/w0AFAACAAEAAADz/xAAFAACAAEAAAD2/xEAFAACAAEAAAD3/xEAFAACAAEAAAACABEAFAACAAEAAAASABQAFAACAAEAAAAXAA0AFAACAAEAAAAgABMAFAACAAEAAAAjAP//FAACAAEAAAAdAP//FAACAAEAAAAdAAAAFAACAAEAAAAdAAEAFAACAAEAAAAeAP//FAACAAEAAAAeAAAAFAACAAEAAAAeAAEAFAACAAEAAAAfAP//FAACAAEAAAAfAAAAFAACAAEAAAAfAAEAFAACAAEAAAASAP7/FAACAAEAAAANAP//FAACAAEAAADs/xQAFAAAAB4AAADs/xMAFAADAB4AAADs/xIAFAADAB4AAADs/xEAFAADAB4AAADs/xAAFAADAB4AAADs/w8AFAAAAB4AAADs/w4AFAAAAB4AAADs/w0AFAADAB4AAADs/wwAFAAAAB4AAADs/wsAFAAAAB4AAADs/woAFAADAB4AAADs/wkAFAADAB4AAADs/wgAFAAAAB4AAADs/wcAFAAAAB4AAADs/wYAFAADAB4AAADt/wUAFAAAAB4AAADt/wQAFAAAAB4AAADt/wMAFAAAAB4AAADt/wIAFAAAAB4AAADt/wEAFAAAAB4AAADt/wAAFAAAAB4AAADt////FAADAB4AAADt//7/FAADAB4AAADt//3/FAAAAB4AAADt//z/FAAAAB4AAADt//v/FAAAAB4AAADt//r/FAADAB4AAADt//n/FAAAAB4AAADt//j/FAADAB4AAADn/xQAFAAAAB4AAADn/xMAFAAAAB4AAADo/xIAFAAAAB4AAADo/xEAFAADAB4AAADo/xAAFAADAB4AAADo/w8AFAADAB4AAADp/w4AFAAAAB4AAADp/w0AFAADAB4AAADp/wwAFAADAB4AAADp/wsAFAADAB4AAADq/woAFAAAAB4AAADq/wkAFAADAB4AAADq/wgAFAADAB4AAADq/wcAFAAAAB4AAADr/wYAFAADAB4AAADr/wUAFAAAAB4AAADr/wQAFAAAAB4AAADr/wMAFAADAB4AAADs/wIAFAADAB4AAADs/wEAFAAAAB4AAADs/wAAFAADAB4AAADs////FAADAB4AAADx/xkAFAADAB4AAADx/xgAFAADAB4AAADw/xcAFAAAAB4AAADw/xYAFAADAB4AAADw/xUAFAAAAB4AAADv/xQAFAAAAB4AAADv/xMAFAAAAB4AAADu/xIAFAAAAB4AAADu/xEAFAADAB4AAADu/xAAFAAAAB4AAADt/w8AFAAAAB4AAADt/w4AFAAAAB4AAADt/w0AFAADAB4AAADr/woAFAAAAB4AAADr/wkAFAADAB4AAADr/wgAFAADAB4AAADq/wYAFAAAAB4AAADo/wkAFAAAAB4AAADo/wgAFAAAAB4AAADp/wcAFAADAB4AAADp/wYAFAAAAB4AAADp/wUAFAAAAB4AAADp/wQAFAAAAB4AAADq/wMAFAADAB4AAADq/wIAFAAAAB4AAADq/wEAFAADAB4AAADr/wAAFAAAAB4AAADr////FAADAB4AAADr//7/FAAAAB4AAADr//3/FAAAAB4AAADs//z/FAADAB4AAADs//v/FAADAB4AAADu//f/FAADAB4AAADu//b/FAADAB4AAADv//X/FAAAAB4AAADv//T/FAADAB4AAADw//P/FAADAB4AAADw//L/FAADAB4AAADx//H/FAAAAB4AAADx//D/FAADAB4AAADy/+//FAAAAB4AAADs//n/FAAAAB4AAADv//b/FAAAAB4AAADw//X/FAADAB4AAADx//T/FAAAAB4AAADy//P/FAADAB4AAADv//n/FAADAB4AAADw//n/FAAAAB4AAADx//j/FAAAAB4AAADy//j/FAAAAB4AAADz//f/FAADAB4AAAD0//f/FAAAAB4AAAD1//b/FAAAAB4AAADv//j/FAAAAB4AAADw//f/FAADAB4AAADx//f/FAAAAB4AAADy//b/FAADAB4AAADz//b/FAAAAB4AAAD0//X/FAADAB4AAAD1//X/FAAAAB4AAAD2//T/FAADAB4AAAD3//T/FAADAB4AAAD4//P/FAADAB4AAADz//T/FAAAAB4AAAD0//T/FAAAAB4AAAD1//T/FAAAAB4AAAD4//T/FAADAB4AAAD5//P/FAADAB4AAAD6//P/FAAAAB4AAAD7//P/FAADAB4AAAD8//P/FAAAAB4AAAD9//P/FAADAB4AAAD3//X/FAAAAB4AAAD4//X/FAAAAB4AAAD5//X/FAAAAB4AAAD6//b/FAADAB4AAAD7//b/FAAAAB4AAAD8//b/FAADAB4AAAD6//X/FAADAB4AAAD7//X/FAAAAB4AAAD9//b/FAAAAB4AAAD+//b/FAAAAB4AAAD+//f/FAADAB4AAAD///j/FAADAB4AAAD///f/FAADAB4AAAD9//X/FAADAB4AAAD8//T/FAAAAB4AAAD7//L/FAAAAB4AAAD8//L/FAADAB4AAAD6//H/FAADAB4AAAD5//H/FAADAB4AAAD4//D/FAAAAB4AAAD3//D/FAADAB4AAAD2/+//FAAAAB4AAAD2//D/FAAAAB4AAAD1//D/FAADAB4AAAD0//D/FAADAB4AAADz//D/FAADAB4AAADy//H/FAAAAB4AAADw//H/FAADAB4AAADv//H/FAAAAB4AAADt/woAFAAAAB4AAADu/wkAFAAAAB4AAADu/wgAFAAAAB4AAADv/wcAFAAAAB4AAADv/wYAFAADAB4AAADw/wUAFAADAB4AAADw/wQAFAADAB4AAADv/wMAFAADAB4AAADv/wIAFAADAB4AAADu/wEAFAADAB4AAADs//7/FAADAB4AAAAYAPv/FAADACcAAAAcAPz/FAAFAAYAAAAbAPz/FAAFAAYAAAAaAPz/FAAFAAYAAAAZAPz/FAAFAAYAAAAmAP7/FAAAAB4AAAAmAP//FAAAAB4AAAAmAAAAFAAAAB4AAAAnAAEAFAAAAB4AAAAnAAIAFAAAAB4AAAAnAAMAFAAAAB4AAAAkAAMAFAAAAB4AAAAlAAIAFAAAAB4AAAAlAAEAFAAAAB4AAADy/xUAFAAFAAYAACDz/xUAFAAFAAYAACD0/xUAFAAFAAYAACD1/xUAFAAFAAYAACD2/xUAFAAFAAYAACD3/xUAFAAFAAYAACD4/xUAFAAFAAYAACD5/xUAFAAFAAYAACD6/xUAFAAFAAYAACD7/xUAFAAFAAYAACD8/xUAFAAFAAYAACD9/xUAFAAFAAYAACD+/xUAFAAFAAYAACD//xUAFAAFAAYAACAAABUAFAAFAAYAACABABUAFAAFAAYAACACABUAFAAFAAYAACADABUAFAAFAAYAACAEABUAFAAFAAYAACAFABUAFAAFAAYAACAGABUAFAAFAAYAACAHABUAFAAFAAYAACAIABUAFAAFAAYAACAJABUAFAAFAAYAACAKABUAFAAFAAYAACALABUAFAAFAAYAACAMABUAFAAFAAYAACANABUAFAAFAAYAACAOABUAFAAFAAYAACAPABUAFAAFAAYAACAQABUAFAAFAAYAACARABUAFAAFAAYAACASABUAFAAFAAYAACATABUAFAAFAAYAACAUABUAFAAFAAYAACAVABUAFAAFAAYAACAWABUAFAAFAAYAACAXABUAFAAFAAYAACAYABUAFAAFAAYAACAZABUAFAAFAAYAACAaABUAFAAFAAYAACAbABUAFAAFAAYAACAcABUAFAAFAAYAACAdABUAFAAFAAYAACAeABUAFAAFAAYAACAfABUAFAAFAAYAACAgABUAFAAFAAYAACAhABUAFAAFAAYAACAiABUAFAAFAAYAACAjABUAFAAFAAYAACAkABUAFAAFAAYAACAlABUAFAAFAAYAACAmABUAFAAFAAYAACAmABQAFAAFAAYAAFAmABMAFAAFAAYAAFAmABIAFAAFAAYAAFAmABEAFAAFAAYAAFAmABAAFAAFAAYAAFAmAA8AFAAFAAYAAFAmAA4AFAAFAAYAAFAmAA0AFAAFAAYAAFAmAAwAFAAFAAYAAFAdAPj/FAAEABMAAAAfAPj/FAAAAAIAAAAXABEAFAAGACsAAAARAA4AFAAHACoAABA=")
tile_set = ExtResource("1_8drhf")

[connection signal="solved" from="SequencePuzzle" to="CollectibleItem" method="reveal"]

[editable path="Player"]
"#;

    use crate::diff::text_differ::TextDiff;
    use crate::diff::differ::ChangeType;

    #[test]
    fn test_complete_scene(){
        let source = COMPLEX_SCENE.to_string();
        let scene = parse_scene(&source).expect("parse should succeed");
        let serialized = scene.serialize();
        let round_trip = parse_scene(&serialized).expect("re-parse of serialized output should succeed");
        // println!("{}", serialized);
        assert_eq!(scene.uid, round_trip.uid);
        assert_eq!(scene.nodes.len(), round_trip.nodes.len());
        let diff = if source != serialized {
            let text_diff = TextDiff::create("complex_scene", &source, &serialized, ChangeType::Modified);
            text_diff.print_colorized();
            text_diff.to_unified()
        } else {
            "".to_string()
        };
        assert!(diff.is_empty());
        // assert_eq!(source, serialized);
        
    }
}
use std::collections::{HashMap, HashSet};

use automerge::{Automerge, Patch, Prop};
use godot::{
    builtin::{StringName, Variant},
    classes::ClassDb,
    global::str_to_var,
    meta::ToGodot,
};

use crate::{
    diff::differ::{ChangeType, Differ},
    parser::godot_parser::{GodotNode, GodotScene, TypeOrInstance},
};

/// Represents a diff of a scene, with a scene path and a list of changed nodes.
#[derive(Clone, Debug)]
pub struct SceneDiff {
    /// The path of the scene.
    pub path: String,
    /// The change type for the scene.
    pub change_type: ChangeType,
    /// The nodes changed in this diff.
    pub changed_nodes: Vec<NodeDiff>,
}

impl SceneDiff {
    fn new(path: String, change_type: ChangeType, changed_nodes: Vec<NodeDiff>) -> SceneDiff {
        SceneDiff {
            path,
            change_type,
            changed_nodes,
        }
    }
}

/// Represents a diff of a single node within a scene, with a collection of changed properties.
#[derive(Clone, Debug)]
pub struct NodeDiff {
    /// How the node has been changed.
    pub change_type: ChangeType,
    /// The path of the node within the scene.
    pub node_path: String,
    /// The type of the node.
    pub node_type: String,
    /// The changed properties of the node.
    pub changed_properties: HashMap<String, PropertyDiff>,
}

impl NodeDiff {
    pub fn new(
        change_type: ChangeType,
        node_path: String,
        node_type: String,
        changed_properties: HashMap<String, PropertyDiff>,
    ) -> NodeDiff {
        NodeDiff {
            change_type,
            node_path,
            node_type,
            changed_properties,
        }
    }
}

/// Represents a diff of a single Property within a Node, within a Scene.
#[derive(Clone, Debug)]
pub struct PropertyDiff {
    /// The name of the changed property.
    pub name: String,
    /// The change type of the property.
    pub change_type: ChangeType,
    /// The old value of the property, if it existed.
    pub old_value: Option<Variant>,
    /// The new value of the property, if it still exists.
    pub new_value: Option<Variant>,
}

impl PropertyDiff {
    pub fn new(
        name: String,
        change_type: ChangeType,
        old_value: Option<Variant>,
        new_value: Option<Variant>,
    ) -> Self {
        PropertyDiff {
            name,
            change_type,
            old_value,
            new_value,
        }
    }
}

/// The different types of Godot-recognized string values that can be stored in a Variant.
enum VariantStrValue {
    /// A normal string that doesn't refer to a resource.
    Variant(String),
    /// A Godot resource path string.
    ResourcePath(String),
    /// A Godot sub-resource identifier string.
    SubResourceID(String),
    /// A Godot external resource identifier string.
    ExtResourceID(String),
}

/// Implement the to_string method for this enum
impl std::fmt::Display for VariantStrValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VariantStrValue::Variant(s) => write!(f, "{}", s),
            VariantStrValue::ResourcePath(s) => write!(f, "Resource({})", s),
            VariantStrValue::SubResourceID(s) => write!(f, "SubResource({})", s),
            VariantStrValue::ExtResourceID(s) => write!(f, "ExtResource({})", s),
        }
    }
}

/// Returns the relative object path from path to other.
/// If other doesn't exist inside path, returns None.
fn relative_path(path: &Vec<Prop>, other: &Vec<Prop>) -> Option<Vec<Prop>> {
    let mut remaining_path = other.clone();

    for prop in path.iter() {
        if remaining_path.len() == 0 {
            return None;
        }

        if remaining_path.remove(0) != *prop {
            return None;
        }
    }

    Some(remaining_path)
}

/// Implement scene-related functions on the Differ
impl Differ<'_> {
    /// Generate a [SceneDiff] between the previous and current heads, given a set of patches.
    pub(super) fn get_scene_diff(&self, path: &String, patches: &Vec<Patch>) -> SceneDiff {
        // TODO: Remove patch parsing!
        // Get changed ext/sub/node IDs from the patches
        let mut changed_ext_resource_ids: HashSet<String> = HashSet::new();
        let mut changed_sub_resource_ids: HashSet<String> = HashSet::new();
        let mut changed_node_ids: HashSet<i32> = HashSet::new();
        Self::get_changed_ids_from_patches(
            path,
            patches,
            &mut changed_node_ids,
            &mut changed_ext_resource_ids,
            &mut changed_sub_resource_ids,
        );

        // Get old and new scenes for content comparison
        let old_scene = match self
            .branch_state
            .doc_handle
            .with_doc(|d: &Automerge| GodotScene::hydrate_at(d, &path, &self.prev_heads))
        {
            Ok(scene) => Some(scene),
            Err(_) => None,
        };

        let new_scene = match self
            .branch_state
            .doc_handle
            .with_doc(|d: &Automerge| GodotScene::hydrate_at(d, &path, &self.curr_heads))
        {
            Ok(scene) => Some(scene),
            Err(_) => None,
        };

        let mut node_ids = HashSet::new();
        let mut sub_resource_ids = HashSet::new();
        let mut ext_resource_ids = HashSet::new();

        // Collect all the relevant node IDs, sub resource IDs, and ext resource IDs from both scenes.
        if let Some(ref old_scene) = old_scene {
            Self::get_ids_from_scene(
                old_scene,
                &mut node_ids,
                &mut ext_resource_ids,
                &mut sub_resource_ids,
            );
        }
        if let Some(ref new_scene) = new_scene {
            Self::get_ids_from_scene(
                new_scene,
                &mut node_ids,
                &mut ext_resource_ids,
                &mut sub_resource_ids,
            );
        }

        // For both ext resources and sub resources, track them if they've been added or removed.
        for ext_id in ext_resource_ids.iter() {
            let old_has = old_scene
                .as_ref()
                .map(|scene| scene.ext_resources.contains_key(ext_id))
                .unwrap_or(false);
            let new_has = new_scene
                .as_ref()
                .map(|scene| scene.ext_resources.contains_key(ext_id))
                .unwrap_or(false);

            if (old_has && !new_has) || (!old_has && new_has) {
                changed_ext_resource_ids.insert(ext_id.clone());
            }
        }
        for sub_resource_id in sub_resource_ids.iter() {
            let old_has = old_scene
                .as_ref()
                .map(|scene| scene.sub_resources.contains_key(sub_resource_id))
                .unwrap_or(false);
            let new_has = new_scene
                .as_ref()
                .map(|scene| scene.sub_resources.contains_key(sub_resource_id))
                .unwrap_or(false);

            if (old_has && !new_has) || (!old_has && new_has) {
                changed_sub_resource_ids.insert(sub_resource_id.clone());
            }
        }

        let mut node_diffs = Vec::new();

        // Diff each node
        for node_id in &node_ids {
            // TODO: Currently, we track node diffs by patch.
            // When we remove patch tracking, we'll need to actually compare the contents in get_node_diff
            // and return None if they're the same.
            if !changed_node_ids.contains(node_id) {
                continue;
            }

            let Some(diff) = self.get_node_diff(
                *node_id,
                old_scene.as_ref().and_then(|s| s.get_node(*node_id)),
                new_scene.as_ref().and_then(|s| s.get_node(*node_id)),
                old_scene.as_ref(),
                new_scene.as_ref(),
                &changed_ext_resource_ids,
                &changed_sub_resource_ids,
            ) else {
                continue;
            };

            node_diffs.push(diff);
        }

        SceneDiff::new(
            path.clone(),
            match (old_scene, new_scene) {
                (None, Some(_)) => ChangeType::Added,
                (Some(_), None) => ChangeType::Deleted,
                (_, _) => ChangeType::Modified,
            },
            node_diffs,
        )
    }

    /// Generate a [NodeDiff] between two nodes.
    fn get_node_diff(
        &self,
        node_id: i32,
        old_node: Option<&GodotNode>,
        new_node: Option<&GodotNode>,
        old_scene: Option<&GodotScene>,
        new_scene: Option<&GodotScene>,
        changed_ext_resource_ids: &HashSet<String>,
        changed_sub_resource_ids: &HashSet<String>,
    ) -> Option<NodeDiff> {
        if old_node.is_none() && new_node.is_none() {
            return None;
        }

        let old_class_name = old_node.map(|n| Self::get_class_name(&n.type_or_instance, old_scene));
        let new_class_name = new_node.map(|n| Self::get_class_name(&n.type_or_instance, new_scene));

        let mut changed_properties = HashMap::new();

        // Return early if we changed types; don't diff the props
        // Is this correct?
        if old_class_name != new_class_name {
            return Some(NodeDiff::new(
                ChangeType::Modified,
                // require that one of old_scene or new_scene is valid
                new_scene.or(old_scene)?.get_node_path(node_id),
                new_class_name.or(old_class_name)?,
                changed_properties,
            ));
        }

        // Collect all properties from new and old scenes
        let mut props: HashSet<String> = HashSet::new();
        if let Some(node) = old_node {
            for (key, _) in &node.properties {
                let _ = props.insert(key.to_string());
            }
        }
        if let Some(node) = new_node {
            for (key, _) in &node.properties {
                let _ = props.insert(key.to_string());
            }
        }

        // Iterate through the props
        for prop in &props {
            if let Some(prop_diff) = self.get_property_diff(
                prop,
                old_node,
                new_node,
                old_scene,
                new_scene,
                changed_ext_resource_ids,
                changed_sub_resource_ids,
            ) {
                changed_properties.insert(prop.clone(), prop_diff);
            }
        }

        Some(NodeDiff::new(
            match (old_node, new_node) {
                (None, Some(_)) => ChangeType::Added,
                (Some(_), None) => ChangeType::Deleted,
                (_, _) => ChangeType::Modified,
            },
            new_scene.or(old_scene)?.get_node_path(node_id),
            new_class_name.or(old_class_name)?,
            changed_properties,
        ))
    }

    /// Get a class name [String] from a [TypeOrInstance] and the [GodotScene] it is from.
    fn get_class_name(type_or_instance: &TypeOrInstance, scene: Option<&GodotScene>) -> String {
        match type_or_instance {
            TypeOrInstance::Type(type_name) => type_name.clone(),
            TypeOrInstance::Instance(instance_id) => {
                if let Some(scene) = scene {
                    // strip the "ExtResource(" and ")" from the instance_id
                    let instance_id = instance_id
                        .trim_start_matches("ExtResource(\"")
                        .trim_end_matches("\")");
                    if let Some(ext_resource) = scene.ext_resources.get(instance_id) {
                        return format!("Resource(\"{}\")", ext_resource.path);
                    }
                }
                String::new()
            }
        }
    }

    /// Returns the [VariantStrValue] of a property on a node, or the default value if the property doesn't
    /// exist on the node.
    /// If the node itself doesn't exist, returns [None].
    fn get_value_or_default(prop: &String, node: Option<&GodotNode>) -> Option<VariantStrValue> {
        // If this node never existed, don't provide a value.
        let Some(node) = node else {
            return None;
        };

        let val = node.properties.get(prop).map_or_else(
            ||
			// If the property doesn't exist on the node, calculate the default.
			match &node.type_or_instance {
				TypeOrInstance::Type(class_name) => ClassDb::singleton()
					.class_get_property_default_value(
						&StringName::from(class_name),
						&StringName::from(prop),
					)
					.to_string(),
				// Instance properties are always set, regardless of the default value, so this is always empty
				_ => "".to_string(),
			},
            // Otherwise, get the value from the property.
            |val| val.get_value(),
        );

        Some(Self::get_varstr_value(val))
    }

    /// Returns a [PropertyDiff] comparing the old property value versus the new one.
    /// Returns [None] if neither node is valid, or if the value has not meaningfully changed.
    fn get_property_diff(
        &self,
        prop: &String,
        old_node: Option<&GodotNode>,
        new_node: Option<&GodotNode>,
        old_scene: Option<&GodotScene>,
        new_scene: Option<&GodotScene>,
        changed_ext_resource_ids: &HashSet<String>,
        changed_sub_resource_ids: &HashSet<String>,
    ) -> Option<PropertyDiff> {
        // If neither node is valid, there's no valid property diff.
        if new_node.is_none() && old_node.is_none() {
            return None;
        };

        // It's possible that the prop didn't exist on the old node, but does now, or vice versa.
        // We handle this case just by substituting with the default.
        let old_value = Self::get_value_or_default(prop, old_node);
        let new_value = Self::get_value_or_default(prop, new_node);

        let old = old_value
            .as_ref()
            .map(|v| self.get_prop_value(&v, old_scene, true, prop == "script"));
        let new = new_value
            .as_ref()
            .map(|v| self.get_prop_value(&v, new_scene, false, prop == "script"));

        // If we added or removed the node itself, exit early. This happens if one of the nodes is None.
        let Some(new_value) = new_value else {
            return Some(PropertyDiff::new(
                prop.clone(),
                ChangeType::Deleted,
                old,
                new,
            ));
        };
        let Some(old_value) = old_value else {
            return Some(PropertyDiff::new(prop.clone(), ChangeType::Added, old, new));
        };

        // If the same type, and no resource change, there's no real change.
        if let VariantStrValue::SubResourceID(ref old_value) = old_value
            && let VariantStrValue::SubResourceID(ref new_value) = new_value
        {
            if !changed_sub_resource_ids.contains(old_value)
                && !changed_sub_resource_ids.contains(new_value)
            {
                return None;
            }
        } else if let VariantStrValue::ExtResourceID(ref old_value) = old_value
            && let VariantStrValue::ExtResourceID(ref new_value) = new_value
        {
            if old_value == new_value
                && !changed_ext_resource_ids.contains(old_value)
                && !changed_ext_resource_ids.contains(new_value)
            {
                return None;
            }
        } else if let VariantStrValue::ResourcePath(ref old_value) = old_value
            && let VariantStrValue::ResourcePath(ref new_value) = new_value
        {
            if old_value == new_value {
                return None;
            }
        }

        return Some(PropertyDiff::new(
            prop.clone(),
            ChangeType::Modified,
            old,
            new,
        ));
    }

    /// Returns the diff value of a given prop, within a given scene.
    /// Normally, it's a String. If it's a (non-script) ExtResource or ResourcePath,
    /// it displays the resource content.
    fn get_prop_value(
        &self,
        prop_value: &VariantStrValue,
        scene: Option<&GodotScene>,
        is_old: bool,
        is_script: bool,
    ) -> Variant {
        // Prevent loading script files during the diff and creating issues for the editor
        if is_script {
            return str_to_var("<Script changed>");
        }
        let path;
        match prop_value {
            VariantStrValue::Variant(variant) => {
                return str_to_var(variant);
            }
            VariantStrValue::SubResourceID(sub_resource_id) => {
                return format!("<SubResource {} changed>", sub_resource_id).to_variant();
            }
            VariantStrValue::ResourcePath(resource_path) => {
                path = resource_path;
            }
            VariantStrValue::ExtResourceID(ext_resource_id) => {
                let p = scene.and_then(|scene| {
                    scene
                        .ext_resources
                        .get(ext_resource_id)
                        .map(|ext_resource| &ext_resource.path)
                });
                let Some(p) = p else {
                    return str_to_var("<ExtResource not found>");
                };
                path = &p;
            }
        }

        let Some(resource) = self.load_ext_resource(
            &path,
            if is_old {
                &self.prev_heads
            } else {
                &self.curr_heads
            },
        ) else {
            return str_to_var("<ExtResource not found>");
        };

        return resource;
    }

    /// Populates [node_ids], [ext_resource_ids], and [sub_resource_ids] from the
    /// given scene.
    fn get_ids_from_scene(
        scene: &GodotScene,
        node_ids: &mut HashSet<i32>,
        ext_resource_ids: &mut HashSet<String>,
        sub_resource_ids: &mut HashSet<String>,
    ) {
        for (ext_id, _) in scene.ext_resources.iter() {
            ext_resource_ids.insert(ext_id.clone());
        }
        for (node_id, _) in scene.nodes.iter() {
            node_ids.insert(node_id.clone());
        }
        for (sub_id, _) in scene.sub_resources.iter() {
            sub_resource_ids.insert(sub_id.clone());
        }
    }

    fn get_varstr_value(prop_value: String) -> VariantStrValue {
        if prop_value.starts_with("Resource(")
            || prop_value.starts_with("SubResource(")
            || prop_value.starts_with("ExtResource(")
        {
            let id = prop_value
                .split("(\"")
                .nth(1)
                .unwrap()
                .split("\")")
                .nth(0)
                .unwrap()
                .trim()
                .to_string();
            if prop_value.contains("SubResource(") {
                return VariantStrValue::SubResourceID(id);
            } else if prop_value.contains("ExtResource(") {
                return VariantStrValue::ExtResourceID(id);
            } else {
                // Resource()
                return VariantStrValue::ResourcePath(id);
            }
        }
        // normal variant string
        return VariantStrValue::Variant(prop_value);
    }

    /// Get the changed node IDs, ext resource IDs, and sub resource IDs from a patches array.
    fn get_changed_ids_from_patches(
        path: &String,
        patches: &Vec<Patch>,
        node_ids: &mut HashSet<i32>,
        ext_resource_ids: &mut HashSet<String>,
        sub_resource_ids: &mut HashSet<String>,
    ) {
        let nodes_path = Vec::from([
            Prop::Map(String::from("files")),
            Prop::Map(String::from(path.clone())),
            Prop::Map(String::from("structured_content")),
            Prop::Map(String::from("nodes")),
        ]);

        let ext_resources_path = Vec::from([
            Prop::Map(String::from("files")),
            Prop::Map(String::from(path.clone())),
            Prop::Map(String::from("structured_content")),
            Prop::Map(String::from("ext_resources")),
        ]);

        let sub_resources_path = Vec::from([
            Prop::Map(String::from("files")),
            Prop::Map(String::from(path.clone())),
            Prop::Map(String::from("structured_content")),
            Prop::Map(String::from("sub_resources")),
        ]);

        for patch in patches.iter() {
            let this_path = patch.path.iter().map(|(_, v)| v.clone()).collect();

            // Look for changed nodes
            if let Some(path) = relative_path(&nodes_path, &this_path) {
                if let Some(Prop::Map(node_id)) = path.first() {
                    // hack: only consider nodes where properties changed as changed
                    // this filters out all the parent nodes that don't really change only the child_node_ids change
                    // get second to last instead of last
                    if path.len() > 2 {
                        if let Some(Prop::Map(key)) = path.get(path.len() - 2) {
                            if key == "properties" {
                                node_ids.insert(node_id.parse::<i32>().unwrap());
                                continue;
                            }
                        }
                    }
                    if let Some(Prop::Map(key)) = path.last() {
                        if key != "child_node_ids" {
                            node_ids.insert(node_id.parse::<i32>().unwrap());
                        }
                    }
                };
            }
            // Look for changed ext resources
            else if let Some(path) = relative_path(&ext_resources_path, &this_path) {
                if let Some(Prop::Map(ext_id)) = path.first() {
                    if let Some(Prop::Map(key)) = path.last() {
                        if key != "idx" {
                            // ignore idx changes
                            ext_resource_ids.insert(ext_id.clone());
                        }
                    }
                }
            }
            // Look for changed sub resources
            else if let Some(path) = relative_path(&sub_resources_path, &this_path) {
                if let Some(Prop::Map(sub_id)) = path.first() {
                    if path.len() > 2 {
                        if let Some(Prop::Map(key)) = path.get(path.len() - 2) {
                            if key == "properties" {
                                sub_resource_ids.insert(sub_id.clone());
                                continue;
                            }
                        }
                    }

                    if let Some(Prop::Map(key)) = path.last() {
                        if key != "idx" {
                            // ignore idx changes
                            sub_resource_ids.insert(sub_id.clone());
                        }
                    }
                }
            }
        }
    }
}

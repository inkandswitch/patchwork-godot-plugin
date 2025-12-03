use ::safer_ffi::prelude::*;
use automerge::{Automerge, Patch, PatchAction, Prop};
use automerge::{ChangeHash, ObjId, ObjType, ROOT, ReadDoc};
use automerge_repo::{DocHandle, DocumentId, PeerConnectionInfo};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use godot::classes::resource_loader::CacheMode;
use godot::classes::{ClassDb, ResourceLoader};
use godot::global::str_to_var;
use godot::prelude::Dictionary;
use godot::prelude::*;
use std::path::PathBuf;
use std::time::SystemTime;
use std::{cell::RefCell, collections::HashSet};
use std::{collections::HashMap, str::FromStr};
use tracing::instrument;

use crate::diff::differ::{
    ChangeType, Diff, FileDiff, NodeDiff, ProjectDiff, PropertyDiff, ResourceDiff, SceneDiff,
    TextDiff,
};
use crate::fs::file_system_driver::{FileSystemDriver, FileSystemEvent, FileSystemUpdateEvent};
use crate::fs::file_utils::FileContent;
use crate::helpers::branch::BranchState;
use crate::helpers::doc_utils::SimpleDocReader;
use crate::helpers::utils::{
    CommitInfo, ToShortForm, get_automerge_doc_diff, get_changed_files_vec, summarize_changes,
};
use crate::interop::godot_accessors::{
    EditorFilesystemAccessor, PatchworkConfigAccessor, PatchworkEditorAccessor,
};
use crate::interop::godot_helpers::{ToDict, VariantTypeGetter};
use crate::parser::godot_parser::{GodotNode, GodotScene, TypeOrInstance};
use crate::project::project::Project;
use crate::project::project_api::{BranchViewModel, ChangeViewModel, ProjectViewModel};
use crate::project::project_driver::{
    ConnectionThreadError, DocHandleType, InputEvent, OutputEvent, ProjectDriver,
};

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

// implement the to_string method for this enum
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

struct Differ<'a> {
    project: Project,

    /// The heads we're currently diffing between.
    curr_heads: Vec<ChangeHash>,
    prev_heads: Vec<ChangeHash>,

    /// Cache that stores our loaded ExtResources so far.
    loaded_ext_resources: RefCell<HashMap<String, Variant>>,

    branch_state: &'a BranchState,
}

impl Differ<'_> {
    // Returns Dictionary of schema:
    // Or None if no change.
    // ChangeType: Added/Modified/Deleted
    // { prop_name: String, change_type: ChangeType, old_value : PropValue, new_value : PropValue
    fn get_changed_prop(
        &self,
        prop: &String,
        old_value: Option<&VariantStrValue>,
        new_value: Option<&VariantStrValue>,
    ) -> Option<PropertyDiff> {
        if old_value.is_none() && new_value.is_none() {
            return None;
        }
        let old = old_value.map(|v| self.get_prop_value(v, &old_scene, true, prop == "script"));
        let new = new_value.map(|v| self.get_prop_value(v, &new_scene, false, prop == "script"));

        // If we added or removed, exit early.
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
        if let VariantStrValue::SubResourceID(old_value) = old_value
            && let VariantStrValue::SubResourceID(new_value) = new_value
        {
            if !all_changed_sub_resource_ids.contains(old_value)
                && !all_changed_sub_resource_ids.contains(new_value)
            {
                return None;
            }
        }

        if let VariantStrValue::ExtResourceID(old_value) = old_value
            && let VariantStrValue::ExtResourceID(new_value) = new_value
        {
            if old_value == new_value
                && !all_changed_ext_resource_ids.contains(old_value)
                && !all_changed_ext_resource_ids.contains(new_value)
            {
                return None;
            }
        }

        if let VariantStrValue::ResourcePath(old_value) = old_value
            && let VariantStrValue::ResourcePath(new_value) = new_value
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
    /// If we return a string, it means we display that string. If a diff dict, we display the diff dict.
    fn get_prop_value(
        &self,
        prop_value: &VariantStrValue,
        scene: Option<&GodotScene>,
        is_old: bool,
        is_script: bool,
    ) -> Variant {
        // HACK: prevent loading script files during the diff and creating issues for the editor
        if is_script {
            return str_to_var("<script>");
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

    fn get_ids_from_scene(
        scene: &GodotScene,
        node_ids: &mut HashSet<i32>,
        ext_resource_ids: &mut HashSet<String>,
        sub_resource_ids: &mut HashSet<String>,
    ) {
        for (ext_id, ext_resource) in scene.ext_resources.iter() {
            ext_resource_ids.insert(ext_id.clone());
        }
        for (node_id, _) in scene.nodes.iter() {
            node_ids.insert(node_id.clone());
        }
        for (sub_id, _) in scene.sub_resources.iter() {
            sub_resource_ids.insert(sub_id.clone());
        }
    }

    fn get_resource_at(
        &self,
        path: &String,
        file_content: &FileContent,
        heads: &Vec<ChangeHash>,
    ) -> Option<Variant> {
        let import_path = format!("{}.import", path);
        let mut import_file_content = self.get_file_at(&import_path, Some(heads));
        if import_file_content.is_none() {
            // try at current heads
            import_file_content = self.get_file_at(&import_path, None);
        }
        return self.create_temp_resource_from_content(
            &path,
            &file_content,
            &heads,
            import_file_content.as_ref(),
        );
    }

    fn create_temp_resource_from_content(
        &self,
        path: &str,
        content: &FileContent,
        heads: &Vec<ChangeHash>,
        import_file_content: Option<&FileContent>,
    ) -> Option<Variant> {
        let temp_dir = format!("res://.patchwork/temp_{}/", heads.first().to_short_form());
        let temp_path = path.replace("res://", &temp_dir);
        if let Err(e) = FileContent::write_file_content(
            &PathBuf::from(self.project.globalize_path(&temp_path)),
            content,
        ) {
            tracing::error!("error writing file to temp path: {:?}", e);
            return None;
        }

        if let Some(import_file_content) = import_file_content {
            if let FileContent::String(import_file_content) = import_file_content {
                let import_file_content = import_file_content.replace("res://", &temp_dir);
                // regex to replace uid=uid://<...> and uid=uid://<invalid> with uid=uid://<...> and uid=uid://<invalid>
                let import_file_content =
                    import_file_content.replace(r#"uid=uid://[^\n]+"#, "uid=uid://<invalid>");
                // write the import file content to the temp path
                let import_file_path: String = format!("{}.import", temp_path);
                let _ = FileContent::write_file_content(
                    &PathBuf::from(self.project.globalize_path(&import_file_path)),
                    &FileContent::String(import_file_content),
                );

                let res = PatchworkEditorAccessor::import_and_load_resource(&temp_path);
                if res.is_nil() {
                    tracing::error!("error importing resource: {:?}", temp_path);
                    return None;
                }
                return Some(res);
            }
        }
        let resource = ResourceLoader::singleton()
            .load_ex(&GString::from(temp_path))
            .cache_mode(CacheMode::IGNORE_DEEP)
            .done();
        if let Some(resource) = resource {
            return Some(resource.to_variant());
        }
        None
    }

    fn get_file_at(&self, path: &String, heads: Option<&Vec<ChangeHash>>) -> Option<FileContent> {
        let mut ret: Option<FileContent> = None;
        {
            let files = self
                .project
                .get_files_at(heads, Some(&HashSet::from_iter(vec![path.clone()])));
            for file in files.into_iter() {
                if file.0 == *path {
                    ret = Some(file.1);
                    break;
                } else {
                    panic!(
                        "Returned a file that didn't match the path!?!??!?!?!?!?!?!!? {:?} != {:?}",
                        file.0, path
                    );
                }
            }
        }
        ret
    }

    /// Loads an ExtResource given a path, using a cache.
    fn load_ext_resource(&self, path: &String, heads: &Vec<ChangeHash>) -> Option<Variant> {
        if let Some(resource) = self.loaded_ext_resources.borrow().get(path) {
            return Some(resource.clone());
        }

        let resource_content = self.get_file_at(path, Some(heads));
        let Some(resource_content) = resource_content else {
            return None;
        };

        let Some(resource) = self.get_resource_at(path, &resource_content, heads) else {
            return None;
        };

        return self
            .loaded_ext_resources
            .borrow_mut()
            .insert(path.clone(), resource);
    }

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

    fn get_scene_diff(&self, path: &String, patches: &Vec<Patch>) -> SceneDiff {
        let mut changed_nodes = Array::new();

        // TODO: Remove patch parsing!
        // Get changed ext/sub/node IDs from the patches
        let mut all_changed_ext_resource_ids: HashSet<String> = HashSet::new();
        let mut all_changed_sub_resource_ids: HashSet<String> = HashSet::new();
        let mut changed_node_ids: HashSet<i32> = HashSet::new();
        Self::get_changed_ids_from_patches(
            path,
            patches,
            &mut changed_node_ids,
            &mut all_changed_ext_resource_ids,
            &mut all_changed_sub_resource_ids,
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

        let mut all_node_ids = HashSet::new();
        let mut all_sub_resource_ids = HashSet::new();
        let mut all_ext_resource_ids = HashSet::new();

        // Collect all the relevant node IDs, sub resource IDs, and ext resource IDs.
        if let Some(old_scene) = old_scene {
            Self::get_ids_from_scene(
                &old_scene,
                &mut all_node_ids,
                &mut all_ext_resource_ids,
                &mut all_sub_resource_ids,
            );
        }
        if let Some(new_scene) = new_scene {
            Self::get_ids_from_scene(
                &new_scene,
                &mut all_node_ids,
                &mut all_ext_resource_ids,
                &mut all_sub_resource_ids,
            );
        }

        // For both ext resources and sub resources, track them if they've been added or removed.
        for ext_id in all_ext_resource_ids.iter() {
            let old_has = old_scene
                .map(|scene| scene.ext_resources.contains_key(ext_id))
                .unwrap_or(false);
            let new_has = new_scene
                .map(|scene| scene.ext_resources.contains_key(ext_id))
                .unwrap_or(false);

            if (old_has && !new_has) || (!old_has && new_has) {
                all_changed_ext_resource_ids.insert(ext_id.clone());
            }
        }
        for sub_resource_id in all_sub_resource_ids.iter() {
            let old_has = old_scene
                .map(|scene| scene.sub_resources.contains_key(sub_resource_id))
                .unwrap_or(false);
            let new_has = new_scene
                .map(|scene| scene.sub_resources.contains_key(sub_resource_id))
                .unwrap_or(false);
            if (old_has && !new_has) || (!old_has && new_has) {
                all_changed_sub_resource_ids.insert(sub_resource_id.clone());
            }
        }

        // Handle changed sub resources
        for node_id in all_node_ids.iter() {
            let old_has = old_scene
                .as_ref()
                .map(|scene| scene.nodes.get(node_id).is_some())
                .unwrap_or(false);
            let new_has = new_scene
                .as_ref()
                .map(|scene| scene.nodes.get(node_id).is_some())
                .unwrap_or(false);
            let mut changed_props: Dictionary = Dictionary::new();

            let removed = old_has && !new_has;
            let added = !old_has && new_has;
            if added || removed {
                let Some(scene) = (if added { new_scene } else { old_scene }) else {
                    continue;
                };
                let Some(node) = scene.nodes.get(&node_id.clone()) else {
                    continue;
                };
                let mut changed_props = HashMap::new();
                for (key, value) in node.properties.iter() {
                    let val = Self::get_varstr_value(value.get_value());
                    let prop = if added {
                        self.get_changed_prop(key, None, Some(&val))
                    } else {
                        self.get_changed_prop(key, Some(&val), None)
                    };
                    if let Some(prop) = prop {
                        changed_props.insert(key.clone(), prop.clone());
                    }
                }

                let node_info = NodeDiff::new(
                    if added {
                        ChangeType::Added
                    } else {
                        ChangeType::Deleted
                    },
                    scene.get_node_path(*node_id),
                    Self::get_class_name(&node.type_or_instance, new_scene.as_ref()),
                    changed_props,
                );
            // TODO (Lilith): stopped here
            } else if old_has && new_has && changed_node_ids.contains(node_id) {
                // if let Some(scene) = &new_scene {
                //     let _ = node_info.insert("node_path", scene.get_node_path(*node_id));
                // }
                let mut old_props = Dictionary::new();
                let mut new_props = Dictionary::new();
                let mut old_type: TypeOrInstance = TypeOrInstance::Type(String::new());
                let mut new_type: TypeOrInstance = TypeOrInstance::Type(String::new());
                // Get old and new node content
                if let Some(old_scene) = &old_scene {
                    if let Some(old_node) = old_scene.nodes.get(node_id) {
                        old_type = old_node.type_or_instance.clone();
                    }
                    if let Some(content) = old_scene.get_node(*node_id).map(|n| n.to_dict()) {
                        if let Some(props) = content.get("properties") {
                            old_props = props.to::<Dictionary>();
                        }
                    }
                }

                if let Some(new_scene) = &new_scene {
                    if let Some(new_node) = new_scene.nodes.get(node_id) {
                        new_type = new_node.type_or_instance.clone();
                    }
                    if let Some(content) = new_scene.get_node(*node_id).map(|n| n.to_dict()) {
                        if let Some(props) = content.get("properties") {
                            new_props = props.to::<Dictionary>();
                        }
                    }
                }
                // old_type and new_type
                let old_class_name = Self::get_class_name(&old_type, old_scene.as_ref());
                let new_class_name = Self::get_class_name(&new_type, new_scene.as_ref());

                if old_class_name == new_class_name {
                    let _ = node_info.insert("type", new_class_name);
                    let mut props: HashSet<String> = HashSet::new();
                    for (key, _) in old_props.iter_shared() {
                        let _ = props.insert(key.to_string());
                    }
                    for (key, _) in new_props.iter_shared() {
                        let _ = props.insert(key.to_string());
                    }
                    for prop in props {
                        let changed_prop;
                        {
                            let prop = prop.clone();

                            let default_value = match &new_type {
                                TypeOrInstance::Type(class_name) => ClassDb::singleton()
                                    .class_get_property_default_value(
                                        &StringName::from(class_name),
                                        &StringName::from(&prop),
                                    )
                                    .to_string(),
                                // Instance properties are always set, regardless of the default value, so this is always empty
                                _ => "".to_string(),
                            };

                            let old_prop = if let Some(old_prop) = old_props.get(prop.as_str()) {
                                old_prop.to_string()
                            } else {
                                default_value.clone()
                            };
                            let new_prop = if let Some(new_prop) = new_props.get(prop.as_str()) {
                                new_prop.to_string()
                            } else {
                                default_value.clone()
                            };
                            let old_value = Self::get_varstr_value(old_prop.clone());
                            let new_value: VariantStrValue =
                                Self::get_varstr_value(new_prop.clone());
                            changed_prop =
                                self.get_changed_prop(&prop, Some(&old_value), Some(&new_value));
                        }

                        if let Some(changed_prop) = changed_prop {
                            let _ = changed_props.insert(prop.clone(), changed_prop);
                        }
                    }
                    if changed_props.len() > 0 {
                        let _ = node_info.insert("changed_props", changed_props);
                    }
                    changed_nodes.push(&node_info.to_variant());
                }
            }
        }
        let _ = result.insert("changed_nodes", changed_nodes);
        result
    }

    fn get_node_diff(
        &self,
        node_id: i32,
        old_node: Option<&GodotNode>,
        new_node: Option<&GodotNode>,
        old_scene: Option<&GodotScene>,
        new_scene: Option<&GodotScene>,
    ) -> Option<NodeDiff> {
        // TODO: handle case of added or removed here
        let old_class_name = old_node.map(|n| Self::get_class_name(&n.type_or_instance, old_scene));
        let new_class_name = new_node.map(|n| Self::get_class_name(&n.type_or_instance, new_scene));

        let mut changed_properties = HashMap::new();


        // From now on, we assume both scenes/nodes are valid.
        // Return early if we changed types; don't diff the props
        // Is this correct? Try without
        if old_class_name != new_class_name {
            return Some(NodeDiff::new(
                ChangeType::Modified,
                new_scene?.get_node_path(node_id),
                new_class_name?,
                changed_properties,
            ));
        }

        // Collect all properties from new and old scenes
        let mut props: HashSet<String> = HashSet::new();
        for (key, _) in &old_node?.properties {
            let _ = props.insert(key.to_string());
        }
        for (key, _) in &new_node?.properties {
            let _ = props.insert(key.to_string());
        }

        // Iterate through the props
        for prop in &props {
            // Get the default value for the prop
            // TODO (Lilith): I'm not convinced this works. What about custom types?
            let default_value = match &new_node?.type_or_instance {
                TypeOrInstance::Type(class_name) => ClassDb::singleton()
                    .class_get_property_default_value(
                        &StringName::from(class_name),
                        &StringName::from(prop),
                    )
                    .to_string(),
                // Instance properties are always set, regardless of the default value, so this is always empty
                _ => "".to_string(),
            };

            let old_value = match old_node?.properties.get(prop) {
                Some(prop) => prop.get_value(),
                None => default_value.clone(),
            };
            let new_value = match new_node?.properties.get(prop) {
                Some(prop) => prop.get_value(),
                None => default_value.clone(),
            };
            if let Some(prop_diff) = self.get_changed_prop(
                prop,
                Some(&Self::get_varstr_value(old_value)),
                Some(&Self::get_varstr_value(new_value)),
            ) {
                changed_properties.insert(prop.clone(), prop_diff);
            }
        }

        Some(NodeDiff::new(
            ChangeType::Modified,
            new_scene?.get_node_path(node_id),
            new_class_name?,
            changed_properties,
        ))
    }

    // probably move this to interop...
    // pub fn get_cached_diff(
    //     &self,
    //     heads_before: Vec<ChangeHash>,
    //     heads_after: Vec<ChangeHash>,
    // ) -> Dictionary {
    //     (*self.diff_cache.borrow_mut())
    //         .entry((heads_before.clone(), heads_after.clone()))
    //         .or_insert_with(|| self.get_changes_between(heads_before, heads_after))
    //         .clone()
    // }

    #[instrument(skip_all, level = tracing::Level::DEBUG)]
    pub fn get_changes_between(
        &self,
        old_heads: &Vec<ChangeHash>,
        curr_heads: &Vec<ChangeHash>,
    ) -> ProjectDiff {
        let checked_out_branch_state = match self.project.get_checked_out_branch_state() {
            Some(branch_state) => branch_state,
            None => return ProjectDiff::default(),
        };

        let curr_heads = if curr_heads.len() == 0 {
            &checked_out_branch_state.synced_heads
        } else {
            curr_heads
        };

        tracing::debug!(
            "branch {:?}, getting changes between {} and {}",
            checked_out_branch_state.name,
            old_heads.to_short_form(),
            curr_heads.to_short_form()
        );

        if old_heads == curr_heads {
            tracing::debug!("no changes");
            return ProjectDiff::default();
        }

        let patches: Vec<Patch> = checked_out_branch_state
            .doc_handle
            .with_doc(|d| get_automerge_doc_diff(d, &old_heads, &curr_heads));

        let mut diffs: Vec<Diff> = vec![];
        // Get old and new content
        let new_file_contents = self.project.get_changed_file_content_between(
            None,
            checked_out_branch_state.doc_handle.document_id().clone(),
            old_heads.clone(),
            curr_heads.clone(),
            false,
        );
        let changed_files_set: HashSet<String> = new_file_contents
            .iter()
            .map(|event| match event {
                FileSystemEvent::FileCreated(path, _) => path.to_string_lossy().to_string(),
                FileSystemEvent::FileModified(path, _) => path.to_string_lossy().to_string(),
                FileSystemEvent::FileDeleted(path) => path.to_string_lossy().to_string(),
            })
            .collect::<HashSet<String>>();
        let old_file_contents = self.project.get_files_on_branch_at(
            &checked_out_branch_state,
            Some(&old_heads),
            Some(&changed_files_set),
        );

        for event in &new_file_contents {
            let (path, new_file_content, change_type) = match event {
                FileSystemEvent::FileCreated(path, content) => (path, content, ChangeType::Added),
                FileSystemEvent::FileModified(path, content) => {
                    (path, content, ChangeType::Modified)
                }
                FileSystemEvent::FileDeleted(path) => {
                    (path, &FileContent::Deleted, ChangeType::Deleted)
                }
            };
            let path = path.to_string_lossy().to_string();
            let old_file_content = old_file_contents
                .get(&path)
                .unwrap_or(&FileContent::Deleted);
            let old_content_type = old_file_content.get_variant_type();
            let new_content_type = new_file_content.get_variant_type();
            let old_content = match old_content_type {
                VariantType::NIL => None,
                _ => Some(old_file_content),
            };
            let new_content = match new_content_type {
                VariantType::NIL => None,
                _ => Some(new_file_content),
            };

            if old_content_type == VariantType::OBJECT || new_content_type == VariantType::OBJECT {
                // This is a scene file, so use a scene diff
                diffs.push(Diff::Scene(self.get_scene_diff(&path, &patches)));
            } else if old_content_type != VariantType::STRING
                && new_content_type != VariantType::STRING
            {
                // This is a binary file, so use a resource diff
                diffs.push(Diff::Resource(self.get_resource_diff(
                    &path,
                    change_type,
                    old_content,
                    new_content,
                    &old_heads,
                    &curr_heads,
                )));
            } else if old_content_type != VariantType::PACKED_BYTE_ARRAY
                && new_content_type != VariantType::PACKED_BYTE_ARRAY
            {
                // This is a text file, so do a text diff.
                diffs.push(Diff::Text(self.get_text_diff(
                    &path,
                    change_type,
                    old_content,
                    new_content,
                )));
            } else {
                // We have no idea what type of file this is, so just use a generic file diff.
                diffs.push(Diff::File(FileDiff::new(&path, change_type)));
            }
        }

        ProjectDiff { file_diffs: diffs }
    }

    fn get_resource_diff(
        &self,
        path: &String,
        change_type: ChangeType,
        old_content: Option<&FileContent>,
        new_content: Option<&FileContent>,
        old_heads: &Vec<ChangeHash>,
        curr_heads: &Vec<ChangeHash>,
    ) -> ResourceDiff {
        let import_path = format!("{}.import", path);
        let get_import_file_content = |heads: &Vec<ChangeHash>| -> Option<FileContent> {
            self.get_file_at(&import_path, Some(heads))
                // try at current heads
                .or(self.get_file_at(&import_path, None))
        };

        let old_import_file_content = old_content.and_then(|c| match c {
            FileContent::Deleted => None,
            _ => get_import_file_content(old_heads),
        });

        let new_import_file_content = new_content.and_then(|c| match c {
            FileContent::Deleted => None,
            _ => get_import_file_content(curr_heads),
        });

        ResourceDiff::new(
            change_type,
            old_heads.clone(),
            curr_heads.clone(),
            old_content.cloned(),
            new_content.cloned(),
            old_import_file_content,
            new_import_file_content,
        )
    }

    fn get_text_diff(
        &self,
        path: &String,
        change_type: ChangeType,
        old_content: Option<&FileContent>,
        new_content: Option<&FileContent>,
    ) -> TextDiff {
        let empty_string = String::from("");
        let old_text = if let Some(FileContent::String(s)) = old_content {
            &s
        } else {
            &empty_string
        };
        let new_text = if let Some(FileContent::String(s)) = new_content {
            &s
        } else {
            &empty_string
        };
        TextDiff::create(path, old_text, new_text, change_type)
    }
}

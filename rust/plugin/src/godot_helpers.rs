use std::path::PathBuf;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, fmt::Display};

use automerge::{Change, ChangeHash};
use automerge_repo::{DocumentId, PeerConnectionInfo};
use godot::meta::GodotType;
use godot::{prelude::*, meta::ToGodot, meta::GodotConvert};
// use godot::prelude::{GString, Variant, Dc};
use crate::file_utils::FileContent;
use crate::godot_parser::{GodotNode, TypeOrInstance};
use crate::godot_project_api::SyncStatus;
use crate::utils::{ChangedFile, CommitInfo, MergeMetadata};
use crate::branch::BranchState;
use crate::differ::{DiffLine, DiffHunk, TextDiffFile};
use automerge::{transaction::Transaction, Automerge, ObjId, Prop, ReadDoc, Value};
use godot::builtin::Variant;

pub trait ToRust<T, R> {
	fn to_rust(&self) -> R;
}

// pub trait ToGodotExt<T> {
// 	fn to_godot(&self) -> T;
// 	fn to_variant(&self) -> Variant;
// }


pub trait VariantTypeGetter {
	fn get_variant_type(&self) -> VariantType;
}

pub trait GodotConvertExt {
    /// The type through which `Self` is represented in Godot.
    type Via: GodotType;
}

pub trait ToGodotExt: Sized + GodotConvertExt {
    /// Target type of [`to_godot()`](ToGodot::to_godot), which can differ from [`Via`][GodotConvert::Via] for pass-by-reference types.
    ///
    /// Note that currently, this only differs from `Via` when `Self` is [`RefArg<'r, T>`][crate::meta::RefArg], which is
    /// used inside generated code of  engine methods. Other uses of `to_godot()`, such as return types in `#[func]`, still use value types.
    /// This may change in future versions.
    ///
    /// See also [`AsArg<T>`](crate::meta::AsArg) used as the "front-end" in Godot API parameters.
    type ToVia<'v>: GodotType
    where
        Self: 'v;

    /// Converts this type to the Godot type by reference, usually by cloning.
    fn _to_godot(&self) -> Self::ToVia<'_>;

	fn to_godot(&self) -> Self::ToVia<'_> {
		self._to_godot()
	}

    /// Converts this type to a [Variant].
    // Exception safety: must not panic apart from exceptional circumstances (Nov 2024: only u64).
    // This has invariant implications, e.g. in Array::resize().
    fn _to_variant(&self) -> Variant;

	fn to_variant(&self) -> Variant {
		self._to_variant()
	}
}

fn option_to_godot_tg<TG: ToGodot>(option: &Option<TG>) -> Variant {
	match option {
		Some(s) => s.to_variant(),
		None => Variant::nil(),
	}
}

fn option_to_godot<TG: ToGodotExt>(option: &Option<TG>) -> Variant {
	match option {
		Some(s) => s._to_variant(),
		None => Variant::nil(),
	}
}

pub trait ToVariantExt: Sized {
	fn _to_variant(&self) -> Variant;
	fn to_variant(&self) -> Variant {
		self._to_variant()
	}
}


// packed string array to vector of strings
impl ToRust<PackedStringArray, Vec<String>> for PackedStringArray {
	fn to_rust(&self) -> Vec<String> {
		let mut result = Vec::new();
		for i in 0..self.len() {
			result.push(self.get(i).unwrap().to_string());
		}
		result
	}
}

impl ToRust<PackedStringArray, Vec<ChangeHash>> for PackedStringArray {
	fn to_rust(&self) -> Vec<ChangeHash> {
		let mut result = Vec::new();
		for i in 0..self.len() {
			result.push(ChangeHash::from_str(&self.get(i).unwrap().to_string()).unwrap());
		}
		result
	}
}


impl GodotConvertExt for DocumentId {
	type Via = GString;
}


impl ToGodotExt for DocumentId {
	type ToVia < 'v > = GString;
	fn _to_godot(&self) -> Self::ToVia < '_ > {
		GString::from(self.to_string())
	}
	fn _to_variant(&self) -> Variant {
		self._to_godot().to_variant()
	}
}


fn opt_to_variant(opt: Option<DocumentId>) -> Variant {
    opt.map(|id| id.to_variant()).unwrap_or_default()
}

impl ToVariantExt for Option<DocumentId> {
	fn _to_variant(&self) -> Variant {
		match self {
			Some(id) => id.to_variant(),
			None => Variant::nil(),
		}
	}
}

// impl ToVariantExt for DocumentId {
//     type ToVia<'v> = GString where Self: 'v;
//     fn _to_variant(&self) -> Self::ToVia<'_> {
//         GString::from(self.to_string())
//     }
// }

// impl<T: ToVariantExt> ToVariantExt for &T {
//     type ToVia<'v> = T::ToVia<'v> where Self: 'v;
//     fn _to_variant(&self) -> Self::ToVia<'_> {
//         (*self)._to_variant()
//     }
// }

pub fn get_scene_path_for_node(node: &Gd<Node>) -> String {
	let mut node = node.clone();
	while node.get_scene_file_path().is_empty() && node.get_parent().is_some() {
		node = node.get_parent().unwrap();
	}
	node.get_scene_file_path().to_string()
}

pub fn get_resource_or_scene_path_for_object(object: &Gd<Object>) -> String {
	if let Ok(node) = object.clone().try_cast::<Node>() {
		get_scene_path_for_node(&node)
	} else if let Ok(resource) = object.clone().try_cast::<Resource>() {
		resource.get_path().to_string()
	} else {
		"".to_string()
	}
}

impl<D: Display> GodotConvertExt for Vec<D> {
	type Via = PackedStringArray;
}

impl<D: Display> ToGodotExt for Vec<D> {
	type ToVia<'v> = PackedStringArray where D: 'v;
	fn _to_godot(&self) -> Self::ToVia<'_> {
		self.iter().map(|s| GString::from(s.to_string())).collect()
	}
	fn _to_variant(&self) -> Variant {
		let thingy = self.iter().map(|s| GString::from(s.to_string())).collect::<PackedStringArray>();
		thingy.to_variant()
	}
}

impl GodotConvertExt for PathBuf {
	type Via = GString;
}

impl ToGodotExt for PathBuf {
	type ToVia<'v> = GString where Self: 'v;
	fn _to_godot(&self) -> Self::ToVia<'_> {
		GString::from(self.display().to_string())
	}
	fn _to_variant(&self) -> Variant {
		self._to_godot().to_variant()
	}
}

pub(crate) fn are_valid_heads(packed_string_array: &PackedStringArray) -> bool {
    // check if these are all hex strings
	for h in packed_string_array.to_vec().iter() {
		if !h.to_string().chars().all(|c| c.is_ascii_hexdigit()) {
			return false;
		}
	}
	return true;
}

pub(crate) fn array_to_heads(packed_string_array: PackedStringArray) -> Vec<ChangeHash> {
    packed_string_array
        .to_vec()
        .iter()
        .map(|h| ChangeHash::from_str(h.to_string().as_str()).unwrap())
        .collect()
}

pub(crate) fn heads_to_array(heads: Vec<ChangeHash>) -> PackedStringArray {
    heads
        .iter()
        .map(|h| GString::from(h.to_string()))
        .collect::<PackedStringArray>()
}


impl GodotConvert for MergeMetadata {
	type Via = Dictionary;
}

impl ToGodot for MergeMetadata {
	type ToVia<'v> = Dictionary;
	fn to_godot(&self) -> Dictionary {
		dict! {
			"merged_branch_id": self.merged_branch_id.to_godot(),
			"merged_at_heads": self.merged_at_heads.to_godot(),
			"forked_at_heads": self.forked_at_heads.to_godot(),
		}
	}
	fn to_variant(&self) -> Variant {
				dict! {
			"merged_branch_id": self.merged_branch_id.to_godot(),
			"merged_at_heads": self.merged_at_heads.to_godot(),
			"forked_at_heads": self.forked_at_heads.to_godot(),
		}.to_variant()
	}
}

impl GodotConvert for CommitInfo {
	type Via = Dictionary;
}

impl ToGodot for CommitInfo {
	type ToVia<'v> = Dictionary;
	fn to_godot(&self) -> Dictionary {
		let mut md = dict! {
			"hash": self.hash.to_string().to_godot(),
			"timestamp": self.timestamp.to_godot(),
		};
		if let Some(metadata) = &self.metadata {
			if let Some(username) = &metadata.username {
				let _ = md.insert("username", username.to_godot());
			}
			if let Some(branch_id) = &metadata.branch_id {
				let _ = md.insert("branch_id", branch_id.to_godot());
			}
			if let Some(merge_metadata) = &metadata.merge_metadata {
				let _ = md.insert("merge_metadata", merge_metadata.to_godot());
			}
			if let Some(reverted_to) = &metadata.reverted_to {
				let _ = md.insert("reverted_to", reverted_to.to_godot());
			}
            if let Some(changed_files) = &metadata.changed_files {
                let _ = md.insert("changed_files", changed_files.to_godot());
            }
		}
		md
	}
	fn to_variant(&self) -> Variant {
		self.to_godot().to_variant()
	}
}


fn branch_state_to_dict(branch_state: &BranchState) -> Dictionary {
    let mut branch = dict! {
        "name": branch_state.name.clone(),
        "id": branch_state.doc_handle.document_id().to_string(),
        "is_main": branch_state.is_main,

        // we shouldn't have branches that don't have any changes but sometimes
        // the branch docs are not synced correctly so this flag is used in the UI to
        // indicate that the branch is not loaded and prevent users from checking it out
        "is_not_loaded": branch_state.doc_handle.with_doc(|d| d.get_heads().len() == 0),
        "heads": heads_to_array(branch_state.synced_heads.clone()),
        "is_merge_preview": branch_state.merge_info.is_some(),
		"is_revert_preview": branch_state.revert_info.is_some(),
    };

    if let Some(fork_info) = &branch_state.fork_info {
        let _ = branch.insert("forked_from", fork_info.forked_from.to_string());
        let _ = branch.insert("forked_at", heads_to_array(fork_info.forked_at.clone()));
    }

    if let Some(merge_info) = &branch_state.merge_info {
        let _ = branch.insert("merge_into", merge_info.merge_into.to_string());
        let _ = branch.insert("merge_at", heads_to_array(merge_info.merge_at.clone()));
    }

	if let Some(created_by) = &branch_state.created_by {
		let _ = branch.insert("created_by", created_by.to_string());
	}

	if let Some(merged_into) = &branch_state.merged_into {
		let _ = branch.insert("merged_into", merged_into.to_string());
	}

	if let Some(reverted_to) = &branch_state.revert_info {
		let _ = branch.insert("reverted_to", heads_to_array(reverted_to.reverted_to.clone()));
	}

    branch
}

impl GodotConvert for BranchState {
	type Via = Dictionary;
}

impl ToGodot for BranchState {
	type ToVia<'v> = Dictionary;
	fn to_godot(&self) -> Dictionary {
		branch_state_to_dict(self)
	}
}

impl ToVariantExt for Option<BranchState> {
	fn _to_variant(&self) -> Variant {
		match self {
			Some(branch_state) => branch_state.to_godot().to_variant(),
			None => Variant::nil(),
		}
	}
}

impl ToVariantExt for Option<&BranchState> {
	fn _to_variant(&self) -> Variant {
		match self {
			Some(branch_state) => branch_state.to_godot().to_variant(),
			None => Variant::nil(),
		}
	}
}

impl GodotConvert for SyncStatus {
	type Via = Dictionary;
}

impl ToGodot for SyncStatus {
	type ToVia<'v> = Dictionary;
	
	fn to_godot(&self) -> Dictionary {
		dict! {
			"state": match self {
				SyncStatus::Unknown => "unknown",
				SyncStatus::Disconnected(_) => "disconnected",
				SyncStatus::UpToDate => "up_to_date",
				SyncStatus::Syncing => "syncing"
			},
			"unsynced_changes": match self {
				SyncStatus::Disconnected(num) => *num as i32,
				_ => 0
			}
		}
	}
}

fn system_time_to_variant(time: SystemTime) -> Variant {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs().to_variant())
        .unwrap_or(Variant::nil())
}

impl GodotConvert for FileContent {
	type Via = Variant;
}

impl ToGodot for FileContent {
	type ToVia < 'v > = Variant;
	fn to_godot(&self) -> Self::ToVia < '_ > {
		// < Self as crate::obj::EngineBitfield > ::ord(* self)
		self.to_variant()
	}
	fn to_variant(&self) -> Variant {
		match self {
			FileContent::String(s) => GString::from(s).to_variant(),
			FileContent::Binary(bytes) => PackedByteArray::from(bytes.as_slice()).to_variant(),
			FileContent::Scene(scene) => scene.serialize().to_variant(),
			FileContent::Deleted => Variant::nil(),
		}
	}
}

impl VariantTypeGetter for FileContent {
	fn get_variant_type(&self) -> VariantType {
		match self {
			FileContent::String(_) => VariantType::STRING,
			FileContent::Binary(_) => VariantType::PACKED_BYTE_ARRAY,
			FileContent::Scene(_) => VariantType::OBJECT,
			FileContent::Deleted => VariantType::NIL,
		}
	}
}


pub trait ToDict {
	fn to_dict(&self) -> Dictionary;
}

impl ToDict for GodotNode {
	fn to_dict(&self) -> Dictionary {
		let mut content = Dictionary::new();
        // Add basic node properties
        content.insert("name", self.name.clone());

        // Add type or instance
        match &self.type_or_instance {
            TypeOrInstance::Type(type_name) => {
                content.insert("type", type_name.clone());
            }
            TypeOrInstance::Instance(instance_id) => {
                content.insert("instance", instance_id.clone());
            }
        }

        // Add optional properties
        if let Some(owner) = &self.owner {
            content.insert("owner", owner.clone());
        }
        if let Some(index) = self.index {
            content.insert("index", index);
        }
        if let Some(groups) = &self.groups {
            content.insert("groups", groups.clone());
        }

        // Add node properties as a nested dictionary
        let mut properties = Dictionary::new();
        for (key, property) in &self.properties {
            properties.insert(key.clone(), property.value.clone());
        }
        content.insert("properties", properties);

        content
	}
}

pub trait VariantDocReader {
    fn get_variant<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<Variant>;
}

impl VariantDocReader for Automerge {
    fn get_variant<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<Variant> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::F64(num) => Some(Variant::from(num)),
                automerge::ScalarValue::Int(num) => Some(Variant::from(num)),
                automerge::ScalarValue::Str(smol_str) => Some(Variant::from(smol_str.to_string())),
                automerge::ScalarValue::Boolean(bool) => Some(Variant::from(bool)),
                _ => None,
            },
            _ => None,
        }
    }

}

impl VariantDocReader for Transaction<'_> {
    fn get_variant<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<Variant> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::F64(num) => Some(Variant::from(num)),
                automerge::ScalarValue::Int(num) => Some(Variant::from(num)),
                automerge::ScalarValue::Str(smol_str) => Some(Variant::from(smol_str.to_string())),
                automerge::ScalarValue::Boolean(bool) => Some(Variant::from(bool)),
                _ => None,
            },
            _ => None,
        }
    }
}


impl GodotConvertExt for Vec<ChangedFile> {
	type Via = Array<PackedStringArray>;
}

impl ToGodotExt for Vec<ChangedFile> {
	type ToVia<'v> = Array<PackedStringArray>;
	fn _to_godot(&self) -> Array<PackedStringArray> {
        self.iter().map(|s| {
            let mut inner_array = PackedStringArray::new();
            inner_array.push(&s.path.to_godot());
            inner_array.push(&s.change_type.to_string().to_godot());
            return inner_array;
        }).collect::<Array<PackedStringArray>>()
	}
	fn _to_variant(&self) -> Variant {
        self._to_godot().to_variant()
	}
}

impl GodotConvert for DiffLine {
	type Via = Dictionary;
}

impl ToGodot for DiffLine {
	type ToVia<'v> = Dictionary where Self: 'v;
	fn to_godot(&self) -> Self::ToVia<'_> {
		dict! {
			"new_line_no": self.new_line_no,
			"old_line_no": self.old_line_no,
			"content": self.content.to_godot(),
			"status": self.status.to_godot(),
		}
	}
	fn to_variant(&self) -> Variant {
		self.to_godot().to_variant()
	}
}

impl GodotConvert for DiffHunk {
	type Via = Dictionary;
}

impl ToGodot for DiffHunk {
	type ToVia<'v> = Dictionary where Self: 'v;
	fn to_godot(&self) -> Self::ToVia<'_> {
		dict! {
			"new_start": self.new_start,
			"old_start": self.old_start,
			"new_lines": self.new_lines,
			"old_lines": self.old_lines,
			"diff_lines": self.diff_lines.iter().map(|line| line.to_godot()).collect::<Array<Dictionary>>(),
		}
	}
	fn to_variant(&self) -> Variant {
		self.to_godot().to_variant()
	}
}

impl GodotConvert for TextDiffFile {
	type Via = Dictionary;
}

impl ToGodot for TextDiffFile {
	type ToVia<'v> = Dictionary where Self: 'v;
	fn to_godot(&self) -> Self::ToVia<'_> {
		dict! {
			"new_file": self.new_file.to_godot(),
			"old_file": self.old_file.to_godot(),
			"diff_hunks": self.diff_hunks.iter().map(|hunk| hunk.to_godot()).collect::<Array<Dictionary>>(),
		}
	}
	fn to_variant(&self) -> Variant {
		self.to_godot().to_variant()
	}
}

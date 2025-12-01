use std::path::PathBuf;
use std::{fmt::Display};

use automerge::{ChangeHash};
use automerge_repo::{DocumentId};
use godot::meta::GodotType;
use godot::{prelude::*, meta::ToGodot, meta::GodotConvert};
// use godot::prelude::{GString, Variant, Dc};
use crate::fs::file_utils::FileContent;
use crate::parser::godot_parser::{GodotNode, TypeOrInstance};
use crate::project::godot_project_api::{BranchViewModel, ChangeViewModel, DiffViewModel, SyncStatus};
use crate::helpers::utils::{ChangedFile};
use crate::diff::differ::{DiffLine, DiffHunk, TextDiffFile};
use godot::builtin::Variant;

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

pub trait ToVariantExt: Sized {
	fn _to_variant(&self) -> Variant;
	fn to_variant(&self) -> Variant {
		self._to_variant()
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

impl ToVariantExt for Option<DocumentId> {
	fn _to_variant(&self) -> Variant {
		match self {
			Some(id) => id.to_variant(),
			None => Variant::nil(),
		}
	}
}

impl GodotConvertExt for ChangeHash {
	type Via = GString;
}

impl ToGodotExt for ChangeHash {
	type ToVia < 'v > = GString;
	fn _to_godot(&self) -> Self::ToVia < '_ > {
		GString::from(self.to_string())
	}
	fn _to_variant(&self) -> Variant {
		self._to_godot().to_variant()
	}
}

impl ToVariantExt for Option<ChangeHash> {
	fn _to_variant(&self) -> Variant {
		match self {
			Some(id) => id.to_variant(),
			None => Variant::nil(),
		}
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

// I couldn't figure out how to use GodotConvert with impls, so just use methods for these.

pub(crate) fn branch_view_model_to_dict(branch: &impl BranchViewModel) -> Dictionary {
	let merge_into = branch.get_merge_into();
	let var = merge_into.to_variant();
	dict! {
		"id": branch.get_id().to_godot(),
		"name": branch.get_name(),
		"parent": branch.get_parent().to_variant(),
		"children": branch.get_children().to_godot(),
		"is_available": branch.is_available(),
		"is_loaded": branch.is_loaded(),
		// todo: figure out how to make to_godot work for this
		"reverted_to": branch.get_reverted_to().to_variant(),
		"merge_into": var
	}
}

pub(crate) fn diff_view_model_to_dict(diff: &impl DiffViewModel) -> Dictionary {
	dict! {
		"dict": diff.get_dict().to_godot(),
		"title": diff.get_title().to_godot()
	}
}

pub(crate) fn change_view_model_to_dict(change: &impl ChangeViewModel) -> Dictionary {
	dict! {
		"hash": change.get_hash().to_string(),
		"username": change.get_username(),
		"is_synced": change.is_synced(),
		"summary": change.get_summary(),
		"is_merge": change.is_merge(),
		"merge_id": change.get_merge_id().to_variant(),
		"is_setup": change.is_setup(),
		"exact_timestamp": change.get_exact_timestamp(),
		"human_timestamp": change.get_human_timestamp(),
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
        let _ = content.insert("name", self.name.clone());

        // Add type or instance
        match &self.type_or_instance {
            TypeOrInstance::Type(type_name) => {
                let _ = content.insert("type", type_name.clone());
            }
            TypeOrInstance::Instance(instance_id) => {
                let _ = content.insert("instance", instance_id.clone());
            }
        }

        // Add optional properties
        if let Some(owner) = &self.owner {
            let _ = content.insert("owner", owner.clone());
        }
        if let Some(index) = self.index {
            let _ = content.insert("index", index);
        }
        if let Some(groups) = &self.groups {
            let _ = content.insert("groups", groups.clone());
        }

        // Add node properties as a nested dictionary
        let mut properties = Dictionary::new();
        for (key, property) in &self.properties {
            let _ = properties.insert(key.clone(), property.value.clone());
        }
        let _ = content.insert("properties", properties);

        content
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

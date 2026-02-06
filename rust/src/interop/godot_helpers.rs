use std::path::PathBuf;
use std::{fmt::Display};
use automerge::{ChangeHash};
use godot::classes::ClassDb;
use godot::global::str_to_var;
use samod::{DocumentId};
use godot::meta::{ArgPassing, ByValue, GodotType, ToArg};
use godot::{prelude::*, meta::ToGodot, meta::GodotConvert};
use crate::fs::file_utils::FileContent;
use crate::project::project_api::{BranchViewModel, ChangeViewModel, DiffViewModel, SyncStatus};
use crate::helpers::utils::{ChangedFile};
use godot::builtin::Variant;

use crate::parser::variant_parser::{VariantVal, RealT};

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
    type Pass: ArgPassing;

    /// Converts this type to the Godot type by reference, usually by cloning.
    fn _to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass>;

	fn to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
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
	type Pass = ByValue;
	fn _to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
		GString::from(&self.to_string())
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
	type Pass = ByValue;
	fn _to_godot(&self) -> GString {
		GString::from(&self.to_string())
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
	type Pass = ByValue;
	fn _to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
		self.iter().map(|s| GString::from(&s.to_string())).collect()
	}
	fn _to_variant(&self) -> Variant {
		let thingy = self.iter().map(|s| GString::from(&s.to_string())).collect::<PackedStringArray>();
		thingy.to_variant()
	}
}

impl GodotConvertExt for PathBuf {
	type Via = GString;
}

impl ToGodotExt for PathBuf {
	type Pass = ByValue;
	fn _to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
		GString::from(&self.display().to_string())
	}
	fn _to_variant(&self) -> Variant {
		self._to_godot().to_variant()
	}
}

// I couldn't figure out how to use GodotConvert with impls, so just use methods for these.

pub(crate) fn branch_view_model_to_dict(branch: &impl BranchViewModel) -> VarDictionary {
	let merge_into = branch.get_merge_into();
	let var = merge_into.to_variant();
	vdict! {
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

pub(crate) fn diff_view_model_to_dict(diff: &impl DiffViewModel) -> VarDictionary {
	vdict! {
		"dict": diff.get_diff().to_godot(),
		"title": diff.get_title().to_godot()
	}
}

pub(crate) fn change_view_model_to_dict(change: &impl ChangeViewModel) -> VarDictionary {
	vdict! {
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
	type Via = VarDictionary;
}

impl ToGodot for SyncStatus {
	type Pass = ByValue;

	fn to_godot(&self) -> VarDictionary {
		vdict! {
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
	type Pass = ByValue;
	fn to_godot(&self) -> Variant {
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

impl GodotConvertExt for Vec<ChangedFile> {
	type Via = Array<PackedStringArray>;
}

impl ToGodotExt for Vec<ChangedFile> {
	type Pass = ByValue;
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

trait ToRealFloat {
    // if the godot package is compiled with double-precision, this will return a f64, otherwise a f32.
    #[cfg(not(feature = "double-precision"))] // feature check to see if double-precision is enabled.
    fn to_real(&self) -> f32;
    #[cfg(feature = "double-precision")]
    fn to_real(&self) -> f64;
}

impl ToRealFloat for RealT {
    #[cfg(not(feature = "double-precision"))]
    fn to_real(&self) -> f32 {
        self.to_f32()
    }
    #[cfg(feature = "double-precision")]
    fn to_real(&self) -> f64 {
        self.to_f64()
    }
}   

impl GodotConvert for VariantVal {
    type Via = Variant;
}


impl ToGodot for VariantVal {
    type Pass = ByValue;
    fn to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
        match self {
            VariantVal::Nil => Variant::nil(),
            VariantVal::Bool(b) => Variant::from(*b),
            VariantVal::Int(i) => Variant::from(*i),
            VariantVal::Float(f) => Variant::from(*f),
            VariantVal::String(s) => Variant::from(s.as_str()),
            VariantVal::Vector2(x, y) => Vector2::new(x.to_real(), y.to_real()).to_variant(),
            VariantVal::Vector2i(x, y) => Vector2i::new(*x, *y).to_variant(),
            VariantVal::Rect2((x, y), (width, height)) => Rect2::new(Vector2::new(x.to_real(), y.to_real()), Vector2::new(width.to_real(), height.to_real())).to_variant(),
            VariantVal::Rect2i((x, y), (width, height)) => Rect2i::new(Vector2i::new(*x, *y), Vector2i::new(*width, *height)).to_variant(),
            VariantVal::Vector3(x, y, z) => Vector3::new(x.to_real(), y.to_real(), z.to_real()).to_variant(),
            VariantVal::Vector3i(x, y, z) => Vector3i::new(*x, *y, *z).to_variant(),
            VariantVal::Transform2d( (m00, m01), (m10, m11), (translation_x, translation_y) ) => Transform2D::from_cols(Vector2::new(m00.to_real(), m01.to_real()), Vector2::new(m10.to_real(), m11.to_real()), Vector2::new(translation_x.to_real(), translation_y.to_real())).to_variant(),
            VariantVal::Vector4(x, y, z, w) => Vector4::new(x.to_real(), y.to_real(), z.to_real(), w.to_real()).to_variant(),
            VariantVal::Vector4i(x, y, z, w) => Vector4i::new(*x, *y, *z, *w).to_variant(),
            VariantVal::Plane( (x, y, z), w ) => Plane::new(Vector3::new(x.to_real(), y.to_real(), z.to_real()), w.to_real()).to_variant(),
            VariantVal::Quaternion( x, y, z, w ) => Quaternion::new(x.to_real(), y.to_real(), z.to_real(), w.to_real()).to_variant(),
            VariantVal::Aabb((x, y, z), (width, height, depth)) => Aabb::new(Vector3::new(x.to_real(), y.to_real(), z.to_real()), Vector3::new(width.to_real(), height.to_real(), depth.to_real())).to_variant(),
            VariantVal::Basis((m00, m01, m02), (m10, m11, m12), (m20, m21, m22)) => Basis::from_cols(Vector3::new(m00.to_real(), m01.to_real(), m02.to_real()), Vector3::new(m10.to_real(), m11.to_real(), m12.to_real()), Vector3::new(m20.to_real(), m21.to_real(), m22.to_real())).to_variant(),
            VariantVal::Transform3d(((m00, m01, m02), (m10, m11, m12), (m20, m21, m22)), (origin_x, origin_y, origin_z)) => Transform3D::from_cols(Vector3::new(m00.to_real(), m01.to_real(), m02.to_real()), Vector3::new(m10.to_real(), m11.to_real(), m12.to_real()), Vector3::new(m20.to_real(), m21.to_real(), m22.to_real()), Vector3::new(origin_x.to_real(), origin_y.to_real(), origin_z.to_real())).to_variant(),
            VariantVal::Projection(
                (x0, y0, z0, w0),
                (x1, y1, z1, w1),
                (x2, y2, z2, w2),
                (x3, y3, z3, w3)
            ) => Projection::new(
                [Vector4::new(x0.to_real(), y0.to_real(), z0.to_real(), w0.to_real()), 
                Vector4::new(x1.to_real(), y1.to_real(), z1.to_real(), w1.to_real()), 
                Vector4::new(x2.to_real(), y2.to_real(), z2.to_real(), w2.to_real()), 
                Vector4::new(x3.to_real(), y3.to_real(), z3.to_real(), w3.to_real())]).to_variant(),
            VariantVal::Color(r, g, b, a) => Color::from_rgba(*r, *g, *b, *a).to_variant(),
            VariantVal::StringName(s) => StringName::from(s).to_variant(),
            VariantVal::NodePath(s) => NodePath::from(s).to_variant(),
            VariantVal::Rid(s) => Variant::from(s.as_str()),
            VariantVal::Object(type_, properties) => {
                let instance = ClassDb::singleton().instantiate(type_.as_str());
                let obj = instance.try_to::<Gd<godot::classes::Object>>();
                if let Ok(mut obj) = obj {
                    for (key, value) in properties {
                        obj.set(key.as_str(), &value.to_godot());
                    }
                    obj.to_variant()
                } else {
                    Variant::nil()
                }
            },
            VariantVal::Callable => str_to_var("Callable()"), // godot-rust doesn't expose a way to construct a default callable.
            VariantVal::Signal => str_to_var("Signal()"), // godot-rust doesn't expose a way to construct a default signal.
            VariantVal::Dictionary(dict_type, map) => {
                if let Some((key_type, value_type)) = dict_type {
                    // TODO
                    Variant::nil()
                } else {
                    let mut dict = vdict! {};
                    for (key, value) in map {
                        dict.set(key.to_godot(), value.to_godot());
                    }
                    dict.to_variant()                
				}
            },
            VariantVal::Array(type_, array) => {
                if let Some(type_) = type_ {
                    // TODO
                    Variant::nil()
                } else {
                    let mut godot_array = varray! {};
                    for value in array {
                        godot_array.push(&value.to_godot().to_variant());
                    }
                    godot_array.to_variant()
                }
            }
            VariantVal::PackedByteArray(bytes) => PackedByteArray::from(bytes.as_slice()).to_variant(),
            VariantVal::PackedInt32Array(array) => PackedInt32Array::from(array.as_slice()).to_variant(),
            VariantVal::PackedInt64Array(array) => PackedInt64Array::from(array.as_slice()).to_variant(),
            VariantVal::PackedFloat32Array(array) => PackedFloat32Array::from(array.as_slice()).to_variant(),
            VariantVal::PackedFloat64Array(array) => PackedFloat64Array::from(array.as_slice()).to_variant(),
            VariantVal::PackedStringArray(array) => array.iter().map(|s| GString::from(s)).collect::<PackedStringArray>().to_variant(),
            VariantVal::PackedVector2Array(array) => array.iter().map(|(x, y)| Vector2::new(x.to_real(), y.to_real())).collect::<PackedVector2Array>().to_variant(),
            VariantVal::PackedVector3Array(array) => array.iter().map(|(x, y, z)| Vector3::new(x.to_real(), y.to_real(), z.to_real())).collect::<PackedVector3Array>().to_variant(),
            VariantVal::PackedColorArray(array) => array.iter().map(|(r, g, b, a)| Color::from_rgba(r.to_real(), g.to_real(), b.to_real(), a.to_real())).collect::<PackedColorArray>().to_variant(),
            VariantVal::PackedVector4Array(array) => array.iter().map(|(x, y, z, w)| Vector4::new(x.to_real(), y.to_real(), z.to_real(), w.to_real())).collect::<PackedVector4Array>().to_variant(),
            // TODO: this.
            VariantVal::Resource(uid, path) => Variant::nil(),
            VariantVal::SubResource(s) => Variant::nil(),
            VariantVal::ExtResource(id, uid, path) => Variant::nil(),
        }
    }
}
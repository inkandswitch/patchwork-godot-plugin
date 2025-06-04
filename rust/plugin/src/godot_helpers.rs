use std::path::PathBuf;
use std::str::FromStr;
use std::{collections::HashMap, fmt::Display};

use automerge::{Change, ChangeHash};
use automerge_repo::DocumentId;
use godot::meta::GodotType;
use godot::{prelude::*, meta::ToGodot};
// use godot::prelude::{GString, Variant, Dc};
use crate::file_utils::FileContent;



pub trait ToRust<T, R> {
	fn to_rust(&self) -> R;
}

// pub trait ToGodotExt<T> {
// 	fn to_godot(&self) -> T;
// 	fn to_variant(&self) -> Variant;
// }

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
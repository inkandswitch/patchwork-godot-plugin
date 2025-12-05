use std::collections::HashMap;

use godot::{
    builtin::{Array, Dictionary, GString, Variant, vdict},
    meta::{GodotConvert, ToGodot},
};

use crate::{
    diff::{
        differ::{ChangeType, Diff, ProjectDiff},
        resource_differ::ResourceDiff,
        scene_differ::{NodeDiff, PropertyDiff, SceneDiff},
        text_differ::{TextDiff, TextDiffHunk, TextDiffLine},
    },
    interop::godot_helpers::{GodotConvertExt, ToGodotExt},
};

impl GodotConvert for TextDiffLine {
    type Via = Dictionary;
}

impl ToGodot for TextDiffLine {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;
    fn to_godot(&self) -> Self::ToVia<'_> {
        vdict! {
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

impl GodotConvert for TextDiffHunk {
    type Via = Dictionary;
}

impl ToGodot for TextDiffHunk {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;
    fn to_godot(&self) -> Self::ToVia<'_> {
        vdict! {
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

impl GodotConvert for TextDiff {
    type Via = Dictionary;
}

impl ToGodot for TextDiff {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;
    fn to_godot(&self) -> Self::ToVia<'_> {
        vdict! {
			"path": self.path.to_godot(),
			"diff_type": "text_changed",
			"change_type": self.change_type.to_godot(),
			"text_diff": vdict! {
				// In the future, if we track renames, we should use the different paths here. Currently we don't, though.
				"new_file": self.path.to_godot(),
				"old_file": self.path.to_godot(),
				"diff_hunks": self.diff_hunks.iter().map(|hunk| hunk.to_godot()).collect::<Array<Dictionary>>(),
			}
		}
    }
    fn to_variant(&self) -> Variant {
        self.to_godot().to_variant()
    }
}

impl GodotConvert for ChangeType {
    type Via = GString;
}

impl ToGodot for ChangeType {
    type ToVia<'v>
        = GString
    where
        Self: 'v;

    fn to_godot(&self) -> Self::ToVia<'_> {
        match self {
            ChangeType::Added => "added",
            ChangeType::Modified => "modified",
            ChangeType::Removed => "removed",
        }
        .into()
    }
}

impl GodotConvert for Diff {
    type Via = Dictionary;
}

impl ToGodot for Diff {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;

    fn to_godot(&self) -> Self::ToVia<'_> {
        match self {
            Diff::Scene(diff) => diff.to_godot(),
            Diff::Resource(diff) => diff.to_godot(),
            Diff::Text(diff) => diff.to_godot(),
        }
    }
}

impl GodotConvert for ProjectDiff {
    type Via = Dictionary;
}

impl ToGodot for ProjectDiff {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;

    fn to_godot(&self) -> Self::ToVia<'_> {
        let mut dict = vdict! {};
        for diff in &self.file_diffs {
            dict.set(
                match diff {
                    Diff::Scene(scene_diff) => scene_diff.path.clone(),
                    Diff::Resource(resource_diff) => resource_diff.path.clone(),
                    Diff::Text(text_diff) => text_diff.path.clone(),
                }
                .to_godot(),
                diff.to_godot(),
            )
        }
        dict
    }
}

impl GodotConvert for SceneDiff {
    type Via = Dictionary;
}

impl ToGodot for SceneDiff {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;

    fn to_godot(&self) -> Self::ToVia<'_> {
        vdict! {
            "change_type": self.change_type.to_godot(),
            "changed_nodes": self.changed_nodes.to_godot(),
			"diff_type": "scene_changed"
        }
    }
}

impl GodotConvertExt for Vec<NodeDiff> {
    type Via = Array<Dictionary>;
}

impl ToGodotExt for Vec<NodeDiff> {
    type ToVia<'v> = Array<Dictionary>;
    fn _to_godot(&self) -> Array<Dictionary> {
        self.iter()
            .map(|s| s.to_godot())
            .collect::<Array<Dictionary>>()
    }
    fn _to_variant(&self) -> Variant {
        self._to_godot().to_variant()
    }
}

impl GodotConvert for NodeDiff {
    type Via = Dictionary;
}

impl ToGodot for NodeDiff {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;
    fn to_godot(&self) -> Self::ToVia<'_> {
        vdict! {
            "change_type": self.change_type.to_godot(),
            "changed_props": self.changed_properties.to_godot(),
            "node_path": self.node_path.to_godot(),
            "type": self.node_type.to_godot()
        }
    }
}

impl GodotConvertExt for HashMap<String, PropertyDiff> {
    type Via = Dictionary;
}

impl ToGodotExt for HashMap<String, PropertyDiff> {
    type ToVia<'v> = Dictionary;
    fn _to_godot(&self) -> Dictionary {
        let mut dict = vdict! {};
        for (name, diff) in self {
            dict.set(name.clone(), diff.to_godot());
        }
        dict
    }
    fn _to_variant(&self) -> Variant {
        self._to_godot().to_variant()
    }
}

impl GodotConvert for PropertyDiff {
    type Via = Dictionary;
}

impl ToGodot for PropertyDiff {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;
    fn to_godot(&self) -> Self::ToVia<'_> {
        vdict! {
            "change_type": self.change_type.to_godot(),
            "name": self.name.to_godot(),
            "new_value": self.new_value.clone().unwrap_or(Variant::nil()),
            "old_value": self.old_value.clone().unwrap_or(Variant::nil()),
        }
    }
}

impl GodotConvert for ResourceDiff {
    type Via = Dictionary;
}

impl ToGodot for ResourceDiff {
    type ToVia<'v>
        = Dictionary
    where
        Self: 'v;

    fn to_godot(&self) -> Self::ToVia<'_> {
        vdict! {
            "change_type": self.change_type.to_godot(),
            "new_resource": self.new_resource.clone().unwrap_or(Variant::nil()),
            "old_resource": self.old_resource.clone().unwrap_or(Variant::nil()),
			"diff_type": "resource_changed"
        }
    }
}

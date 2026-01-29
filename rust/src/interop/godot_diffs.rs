use std::collections::HashMap;

use godot::{
    builtin::{Array, GString, VarDictionary, Variant, vdict},
    meta::{ByValue, GodotConvert, ToArg, ToGodot},
};
use godot::obj::Singleton;

use crate::{
    diff::{
        differ::{ChangeType, ContentLoader, Diff, ProjectDiff},
        resource_differ::BinaryResourceDiff,
        scene_differ::{NodeDiff, PropertyDiff, SceneDiff, SubResourceDiff, TextResourceDiff},
        text_differ::{TextDiff, TextDiffHunk, TextDiffLine},
    },
    interop::godot_helpers::{GodotConvertExt, ToGodotExt}, project::branch_db::HistoryRef,
};

impl GodotConvert for TextDiffLine {
    type Via = VarDictionary;
}

impl ToGodot for TextDiffLine {
    type Pass = ByValue;
    fn to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
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
    type Via = VarDictionary;
}

impl ToGodot for TextDiffHunk {
    type Pass = ByValue;
    fn to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
        vdict! {
            "new_start": self.new_start,
            "old_start": self.old_start,
            "new_lines": self.new_lines,
            "old_lines": self.old_lines,
            "diff_lines": self.diff_lines.iter().map(|line| line.to_godot()).collect::<Array<VarDictionary>>(),
        }
    }
    fn to_variant(&self) -> Variant {
        self.to_godot().to_variant()
    }
}

impl GodotConvert for TextDiff {
    type Via = VarDictionary;
}

impl ToGodot for TextDiff {
    type Pass = ByValue;
    fn to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
        vdict! {
			"path": self.path.to_godot(),
			"diff_type": "text_changed",
			"change_type": self.change_type.to_godot(),
			"text_diff": vdict! {
				// In the future, if we track renames, we should use the different paths here. Currently we don't, though.
				"new_file": self.path.to_godot(),
				"old_file": self.path.to_godot(),
				"diff_hunks": self.diff_hunks.iter().map(|hunk| hunk.to_godot()).collect::<Array<VarDictionary>>(),
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
    type Pass = ByValue;
    fn to_godot(&self) -> ToArg<'_, Self::Via, Self::Pass> {
        match self {
            ChangeType::Added => "added",
            ChangeType::Modified => "modified",
            ChangeType::Removed => "removed",
        }
        .into()
    }
}


trait DiffToGodot{
    fn to_dict(&self, content_loader: &impl ContentLoader, old_ref: &HistoryRef, new_ref: &HistoryRef) -> VarDictionary;
}


impl DiffToGodot for Diff {
    fn to_dict(&self, content_loader: &impl ContentLoader, old_ref: &HistoryRef, new_ref: &HistoryRef) -> VarDictionary {
        match self {
            Diff::Scene(diff) => diff.to_dict(content_loader, old_ref, new_ref),
            Diff::TextResourceDiff(diff) => diff.to_dict(content_loader, old_ref, new_ref),
            Diff::BinaryResource(diff) => diff.to_dict(content_loader, old_ref, new_ref),
            Diff::Text(diff) => diff.to_dict(content_loader, old_ref, new_ref),
        }
    }
}

impl DiffToGodot for BinaryResourceDiff {
    fn to_dict(&self, content_loader: &impl ContentLoader, old_ref: &HistoryRef, new_ref: &HistoryRef) -> VarDictionary {
        vdict! {
            "change_type": self.change_type.to_godot(),
            "new_resource": self.new_content.as_ref().map(|v| content_loader.get_resource(v, old_ref)).unwrap_or(Variant::nil()),
            "old_resource": self.old_content.as_ref().map(|v| content_loader.get_resource(v, new_ref)).unwrap_or(Variant::nil()),
        }
    }
}

impl DiffToGodot for TextDiff {
    fn to_dict(&self, _content_loader: &impl ContentLoader, _old_ref: &HistoryRef, _new_ref: &HistoryRef) -> VarDictionary {
        return self.to_godot();
    }
}

impl DiffToGodot for SceneDiff {
    fn to_dict(&self, content_loader: &impl ContentLoader, old_ref: &HistoryRef, new_ref: &HistoryRef) -> VarDictionary {
        vdict! {
            "change_type": self.change_type.to_godot(),
            "changed_nodes": self.changed_nodes.iter().map(|node| node.to_dict(content_loader, old_ref, new_ref)).collect::<Array<VarDictionary>>(),
            "diff_type": "scene_changed"
        }
    }
}

impl DiffToGodot for PropertyDiff {
    fn to_dict(&self, content_loader: &impl ContentLoader, old_ref: &HistoryRef, new_ref: &HistoryRef) -> VarDictionary {
        let is_script = self.name == "script";
        vdict! {
            "name": self.name.to_godot(),
            "change_type": self.change_type.to_godot(),
            "new_value": self.new_value.as_ref().map(|v| content_loader.get_prop_value(v, is_script, new_ref)).unwrap_or(Variant::nil()),
            "old_value": self.old_value.as_ref().map(|v| content_loader.get_prop_value(v, is_script, old_ref)).unwrap_or(Variant::nil()),
        }
    }
}

impl DiffToGodot for HashMap<String, PropertyDiff> {
    fn to_dict(&self, content_loader: &impl ContentLoader, old_ref: &HistoryRef, new_ref: &HistoryRef) -> VarDictionary {
        let mut dict = vdict! {};
        for (name, diff) in self {
            dict.set(name.clone(), diff.to_dict(content_loader, old_ref, new_ref));
        }
        dict
    }
}


impl DiffToGodot for SubResourceDiff {
    fn to_dict(&self, content_loader: &impl ContentLoader, old_ref: &HistoryRef, new_ref: &HistoryRef) -> VarDictionary {
        vdict! {
            "change_type": self.change_type.to_godot(),
            "sub_resource_id": self.sub_resource_id.to_godot(),
            "resource_type": self.resource_type.to_godot(),
            "changed_props": self.changed_properties.iter().map(|(name, diff)| vdict! {
                "name": name.to_godot(),
                "change_type": diff.change_type.to_godot(),
            }).collect::<Array<VarDictionary>>(),
        }
        
    }
}

impl DiffToGodot for TextResourceDiff {
    fn to_dict(&self, content_loader: &impl ContentLoader, old_ref: &HistoryRef, new_ref: &HistoryRef) -> VarDictionary {
        vdict! {
            "change_type": self.change_type.to_godot(),
            "resource_type": self.resource_type.to_godot(),
            "changed_sub_resources": self.changed_sub_resources.iter().map(|s| s.to_dict(content_loader)).collect::<Array<VarDictionary>>(),
            "changed_main_resource": self.changed_main_resource.as_ref().map(|s| s.to_dict(content_loader)).unwrap_or(Variant::nil()),
            "diff_type": "text_resource_changed",
        }
    }
}
/**************************************************************************/
/*  editor_inspector.cpp                                                  */
/**************************************************************************/
/*                         This file is part of:                          */
/*                             GODOT ENGINE                               */
/*                        https://godotengine.org                         */
/**************************************************************************/
/* Copyright (c) 2014-present Godot Engine contributors (see AUTHORS.md). */
/* Copyright (c) 2007-2014 Juan Linietsky, Ariel Manzur.                  */
/*                                                                        */
/* Permission is hereby granted, free of charge, to any person obtaining  */
/* a copy of this software and associated documentation files (the        */
/* "Software"), to deal in the Software without restriction, including    */
/* without limitation the rights to use, copy, modify, merge, publish,    */
/* distribute, sublicense, and/or sell copies of the Software, and to     */
/* permit persons to whom the Software is furnished to do so, subject to  */
/* the following conditions:                                              */
/*                                                                        */
/* The above copyright notice and this permission notice shall be         */
/* included in all copies or substantial portions of the Software.        */
/*                                                                        */
/* THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,        */
/* EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF     */
/* MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. */
/* IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY   */
/* CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,   */
/* TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE      */
/* SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.                 */
/**************************************************************************/

#include "diff_inspector.h"

#include "core/os/keyboard.h"
#include "editor/doc_tools.h"
#include "editor/editor_feature_profile.h"
#include "editor/editor_node.h"
#include "editor/editor_property_name_processor.h"
#include "editor/editor_settings.h"
#include "editor/editor_string_names.h"
#include "editor/editor_undo_redo_manager.h"
#include "editor/gui/editor_validation_panel.h"
#include "editor/inspector_dock.h"
#include "editor/multi_node_edit.h"
#include "editor/plugins/script_editor_plugin.h"
#include "editor/themes/editor_scale.h"
#include "editor/themes/editor_theme_manager.h"
#include "scene/gui/margin_container.h"
#include "scene/gui/spin_box.h"
#include "scene/gui/texture_rect.h"
#include "scene/property_utils.h"
#include "scene/resources/packed_scene.h"
#include "scene/resources/style_box_flat.h"
#include "scene/scene_string_names.h"

bool DiffInspector::_property_path_matches(const String &p_property_path, const String &p_filter, EditorPropertyNameProcessor::Style p_style) {
	if (p_property_path.containsn(p_filter)) {
		return true;
	}

	const Vector<String> prop_sections = p_property_path.split("/");
	for (int i = 0; i < prop_sections.size(); i++) {
		if (p_filter.is_subsequence_ofn(EditorPropertyNameProcessor::get_singleton()->process_name(prop_sections[i], p_style, p_property_path))) {
			return true;
		}
	}
	return false;
}

////////////////////////////////////////////////
////////////////////////////////////////////////

void DiffInspectorCategory::_notification(int p_what) {
	switch (p_what) {
		case NOTIFICATION_ENTER_TREE:
		case NOTIFICATION_THEME_CHANGED: {
			menu->set_item_icon(menu->get_item_index(MENU_OPEN_DOCS), get_editor_theme_icon(SNAME("Help")));
		} break;
		case NOTIFICATION_DRAW: {
			Ref<StyleBox> sb = get_theme_stylebox(SNAME("bg"));

			draw_style_box(sb, Rect2(Vector2(), get_size()));

			Ref<Font> font = get_theme_font(SNAME("bold"), EditorStringName(EditorFonts));
			int font_size = get_theme_font_size(SNAME("bold_size"), EditorStringName(EditorFonts));

			int hs = get_theme_constant(SNAME("h_separation"), SNAME("Tree"));
			int icon_size = get_theme_constant(SNAME("class_icon_size"), EditorStringName(Editor));

			int w = font->get_string_size(label, HORIZONTAL_ALIGNMENT_LEFT, -1, font_size).width;
			if (icon.is_valid()) {
				w += hs + icon_size;
			}
			w = MIN(w, get_size().width - sb->get_minimum_size().width);

			int ofs = (get_size().width - w) / 2;

			float v_margin_offset = sb->get_content_margin(SIDE_TOP) - sb->get_content_margin(SIDE_BOTTOM);

			if (icon.is_valid()) {
				Size2 rect_size = Size2(icon_size, icon_size);
				Point2 rect_pos = Point2(ofs, (get_size().height - icon_size) / 2 + v_margin_offset).round();
				if (is_layout_rtl()) {
					rect_pos.x = get_size().width - rect_pos.x - icon_size;
				}
				draw_texture_rect(icon, Rect2(rect_pos, rect_size));

				ofs += hs + icon_size;
				w -= hs + icon_size;
			}

			Color color = get_theme_color(SceneStringName(font_color), SNAME("Tree"));
			if (is_layout_rtl()) {
				ofs = get_size().width - ofs - w;
			}
			float text_pos_y = font->get_ascent(font_size) + (get_size().height - font->get_height(font_size)) / 2 + v_margin_offset;
			Point2 text_pos = Point2(ofs, text_pos_y).round();
			draw_string(font, text_pos, label, HORIZONTAL_ALIGNMENT_LEFT, w, font_size, color);
		} break;
	}
}

Control *DiffInspectorCategory::make_custom_tooltip(const String &p_text) const {
	// If it's not a doc tooltip, fallback to the default one.
	if (doc_class_name.is_empty()) {
		return nullptr;
	}

	EditorHelpBit *help_bit = memnew(EditorHelpBit(p_text));
	EditorHelpBitTooltip::show_tooltip(help_bit, const_cast<EditorInspectorCategory *>(this));
	return memnew(Control); // Make the standard tooltip invisible.
}

Size2 DiffInspectorCategory::get_minimum_size() const {
	Ref<Font> font = get_theme_font(SNAME("bold"), EditorStringName(EditorFonts));
	int font_size = get_theme_font_size(SNAME("bold_size"), EditorStringName(EditorFonts));

	Size2 ms;
	ms.height = font->get_height(font_size);
	if (icon.is_valid()) {
		int icon_size = get_theme_constant(SNAME("class_icon_size"), EditorStringName(Editor));
		ms.height = MAX(icon_size, ms.height);
	}
	ms.height += get_theme_constant(SNAME("v_separation"), SNAME("Tree"));

	const Ref<StyleBox> &bg_style = get_theme_stylebox(SNAME("bg"));
	ms.height += bg_style->get_content_margin(SIDE_TOP) + bg_style->get_content_margin(SIDE_BOTTOM);

	return ms;
}

void DiffInspectorCategory::_handle_menu_option(int p_option) {
	switch (p_option) {
		case MENU_OPEN_DOCS:
			ScriptEditor::get_singleton()->goto_help("class:" + doc_class_name);
			EditorNode::get_singleton()->set_visible_editor(EditorNode::EDITOR_SCRIPT);
			break;
	}
}

void DiffInspectorCategory::gui_input(const Ref<InputEvent> &p_event) {
	if (doc_class_name.is_empty()) {
		return;
	}

	const Ref<InputEventMouseButton> &mb_event = p_event;
	if (mb_event.is_null() || !mb_event->is_pressed() || mb_event->get_button_index() != MouseButton::RIGHT) {
		return;
	}

	menu->set_item_disabled(menu->get_item_index(MENU_OPEN_DOCS), !EditorHelp::get_doc_data()->class_list.has(doc_class_name));

	menu->set_position(get_screen_position() + mb_event->get_position());
	menu->reset_size();
	menu->popup();
}

DiffInspectorCategory::EditorInspectorCategory() {
	menu = memnew(PopupMenu);
	menu->connect(SceneStringName(id_pressed), callable_mp(this, &DiffInspectorCategory::_handle_menu_option));
	menu->add_item(TTR("Open Documentation"), MENU_OPEN_DOCS);
	add_child(menu);
}

Ref<EditorInspectorPlugin> DiffInspector::inspector_plugins[MAX_PLUGINS];
int DiffInspector::inspector_plugin_count = 0;

EditorProperty *DiffInspector::instantiate_property_editor(Object *p_object, const Variant::Type p_type, const String &p_path, PropertyHint p_hint, const String &p_hint_text, const uint32_t p_usage, const bool p_wide) {
	for (int i = inspector_plugin_count - 1; i >= 0; i--) {
		if (!inspector_plugins[i]->can_handle(p_object)) {
			continue;
		}

		inspector_plugins[i]->parse_property(p_object, p_type, p_path, p_hint, p_hint_text, p_usage, p_wide);
		if (inspector_plugins[i]->added_editors.size()) {
			for (List<EditorInspectorPlugin::AddedEditor>::Element *E = inspector_plugins[i]->added_editors.front()->next(); E; E = E->next()) { //only keep first one
				memdelete(E->get().property_editor);
			}

			EditorProperty *prop = Object::cast_to<EditorProperty>(inspector_plugins[i]->added_editors.front()->get().property_editor);
			if (prop) {
				inspector_plugins[i]->added_editors.clear();
				return prop;
			} else {
				memdelete(inspector_plugins[i]->added_editors.front()->get().property_editor);
				inspector_plugins[i]->added_editors.clear();
			}
		}
	}
	return nullptr;
}

void DiffInspector::add_inspector_plugin(const Ref<EditorInspectorPlugin> &p_plugin) {
	ERR_FAIL_COND(inspector_plugin_count == MAX_PLUGINS);

	for (int i = 0; i < inspector_plugin_count; i++) {
		if (inspector_plugins[i] == p_plugin) {
			return; //already exists
		}
	}
	inspector_plugins[inspector_plugin_count++] = p_plugin;
}

void DiffInspector::remove_inspector_plugin(const Ref<EditorInspectorPlugin> &p_plugin) {
	ERR_FAIL_COND(inspector_plugin_count == MAX_PLUGINS);

	int idx = -1;
	for (int i = 0; i < inspector_plugin_count; i++) {
		if (inspector_plugins[i] == p_plugin) {
			idx = i;
			break;
		}
	}

	ERR_FAIL_COND_MSG(idx == -1, "Trying to remove nonexistent inspector plugin.");
	for (int i = idx; i < inspector_plugin_count - 1; i++) {
		inspector_plugins[i] = inspector_plugins[i + 1];
	}
	inspector_plugins[inspector_plugin_count - 1] = Ref<EditorInspectorPlugin>();

	inspector_plugin_count--;
}

void DiffInspector::cleanup_plugins() {
	for (int i = 0; i < inspector_plugin_count; i++) {
		inspector_plugins[i].unref();
	}
	inspector_plugin_count = 0;
}

Button *DiffInspector::create_inspector_action_button(const String &p_text) {
	Button *button = memnew(Button);
	button->set_text(p_text);
	button->set_theme_type_variation(SNAME("InspectorActionButton"));
	button->set_h_size_flags(SIZE_SHRINK_CENTER);
	return button;
}

bool DiffInspector::is_main_editor_inspector() const {
	return InspectorDock::get_singleton() && InspectorDock::get_inspector_singleton() == this;
}

String DiffInspector::get_selected_path() const {
	return property_selected;
}

void DiffInspector::_parse_added_editors(VBoxContainer *current_vbox, EditorInspectorSection *p_section, Ref<EditorInspectorPlugin> ped) {
	for (const EditorInspectorPlugin::AddedEditor &F : ped->added_editors) {
		EditorProperty *ep = Object::cast_to<EditorProperty>(F.property_editor);
		current_vbox->add_child(F.property_editor);

		if (ep) {
			ep->object = object;
			ep->connect("property_changed", callable_mp(this, &DiffInspector::_property_changed).bind(false));
			ep->connect("property_keyed", callable_mp(this, &DiffInspector::_property_keyed));
			ep->connect("property_deleted", callable_mp(this, &DiffInspector::_property_deleted), CONNECT_DEFERRED);
			ep->connect("property_keyed_with_value", callable_mp(this, &DiffInspector::_property_keyed_with_value));
			ep->connect("property_checked", callable_mp(this, &DiffInspector::_property_checked));
			ep->connect("property_pinned", callable_mp(this, &DiffInspector::_property_pinned));
			ep->connect("selected", callable_mp(this, &DiffInspector::_property_selected));
			ep->connect("multiple_properties_changed", callable_mp(this, &DiffInspector::_multiple_properties_changed));
			ep->connect("resource_selected", callable_mp(this, &DiffInspector::_resource_selected), CONNECT_DEFERRED);
			ep->connect("object_id_selected", callable_mp(this, &DiffInspector::_object_id_selected), CONNECT_DEFERRED);

			if (F.properties.size()) {
				if (F.properties.size() == 1) {
					//since it's one, associate:
					ep->property = F.properties[0];
					ep->property_path = property_prefix + F.properties[0];
					ep->property_usage = 0;
				}

				if (!F.label.is_empty()) {
					ep->set_label(F.label);
				}

				for (int i = 0; i < F.properties.size(); i++) {
					String prop = F.properties[i];

					if (!editor_property_map.has(prop)) {
						editor_property_map[prop] = List<EditorProperty *>();
					}
					editor_property_map[prop].push_back(ep);
				}
			}

			Node *section_search = p_section;
			while (section_search) {
				EditorInspectorSection *section = Object::cast_to<EditorInspectorSection>(section_search);
				if (section) {
					ep->connect("property_can_revert_changed", callable_mp(section, &EditorInspectorSection::property_can_revert_changed));
				}
				section_search = section_search->get_parent();
				if (Object::cast_to<EditorInspector>(section_search)) {
					// Skip sub-resource inspectors.
					break;
				}
			}

			ep->set_read_only(read_only);
			ep->update_property();
			ep->_update_pin_flags();
			ep->update_editor_property_status();
			ep->set_deletable(deletable_properties);
			ep->update_cache();
		}
	}
	ped->added_editors.clear();
}

bool DiffInspector::_is_property_disabled_by_feature_profile(const StringName &p_property) {
	Ref<EditorFeatureProfile> profile = EditorFeatureProfileManager::get_singleton()->get_current_profile();
	if (profile.is_null()) {
		return false;
	}

	StringName class_name = object->get_class();

	while (class_name != StringName()) {
		if (profile->is_class_property_disabled(class_name, p_property)) {
			return true;
		}
		if (profile->is_class_disabled(class_name)) {
			//won't see properties of a disabled class
			return true;
		}
		class_name = ClassDB::get_parent_class(class_name);
	}

	return false;
}

void DiffInspector::update_tree() {
	// Store currently selected and focused elements to restore after the update.
	// TODO: Can be useful to store more context for the focusable, such as the caret position in LineEdit.
	StringName current_selected = property_selected;
	int current_focusable = -1;
	// Temporarily disable focus following to avoid jumping while the inspector is updating.
	set_follow_focus(false);

	if (property_focusable != -1) {
		// Check that focusable is actually focusable.
		bool restore_focus = false;
		Control *focused = get_viewport() ? get_viewport()->gui_get_focus_owner() : nullptr;
		if (focused) {
			Node *parent = focused->get_parent();
			while (parent) {
				EditorInspector *inspector = Object::cast_to<EditorInspector>(parent);
				if (inspector) {
					restore_focus = inspector == this; // May be owned by another inspector.
					break; // Exit after the first inspector is found, since there may be nested ones.
				}
				parent = parent->get_parent();
			}
		}

		if (restore_focus) {
			current_focusable = property_focusable;
		}
	}

	// Only hide plugins if we are not editing any object.
	// This should be handled outside of the update_tree call anyway (see DiffInspector::edit), but might as well keep it safe.
	_clear(!object);

	if (!object) {
		return;
	}

	List<Ref<EditorInspectorPlugin>> valid_plugins;

	for (int i = inspector_plugin_count - 1; i >= 0; i--) { //start by last, so lastly added can override newly added
		if (!inspector_plugins[i]->can_handle(object)) {
			continue;
		}
		valid_plugins.push_back(inspector_plugins[i]);
	}

	// Decide if properties should be drawn with the warning color (yellow),
	// or if the whole object should be considered read-only.
	bool draw_warning = false;
	bool all_read_only = false;
	if (is_inside_tree()) {
		if (object->has_method("_is_read_only")) {
			all_read_only = object->call("_is_read_only");
		}

		Node *nod = Object::cast_to<Node>(object);
		Node *es = EditorNode::get_singleton()->get_edited_scene();
		if (nod && es != nod && nod->get_owner() != es) {
			// Draw in warning color edited nodes that are not in the currently edited scene,
			// as changes may be lost in the future.
			draw_warning = true;
		} else {
			if (!all_read_only) {
				Resource *res = Object::cast_to<Resource>(object);
				if (res) {
					all_read_only = EditorNode::get_singleton()->is_resource_read_only(res);
				}
			}
		}
	}

	String filter = search_box ? search_box->get_text() : "";
	String group;
	String group_base;
	String subgroup;
	String subgroup_base;
	int section_depth = 0;
	VBoxContainer *category_vbox = nullptr;

	List<PropertyInfo> plist;
	object->get_property_list(&plist, true);

	HashMap<VBoxContainer *, HashMap<String, VBoxContainer *>> vbox_per_path;
	HashMap<String, EditorInspectorArray *> editor_inspector_array_per_prefix;

	Color sscolor = get_theme_color(SNAME("prop_subsection"), EditorStringName(Editor));

	// Get the lists of editors to add the beginning.
	for (Ref<EditorInspectorPlugin> &ped : valid_plugins) {
		ped->parse_begin(object);
		_parse_added_editors(main_vbox, nullptr, ped);
	}

	StringName doc_name;

	// Get the lists of editors for properties.
	for (List<PropertyInfo>::Element *E_property = plist.front(); E_property; E_property = E_property->next()) {
		PropertyInfo &p = E_property->get();

		if (p.usage & PROPERTY_USAGE_SUBGROUP) {
			// Setup a property sub-group.
			subgroup = p.name;

			Vector<String> hint_parts = p.hint_string.split(",");
			subgroup_base = hint_parts[0];
			if (hint_parts.size() > 1) {
				section_depth = hint_parts[1].to_int();
			} else {
				section_depth = 0;
			}

			continue;

		} else if (p.usage & PROPERTY_USAGE_GROUP) {
			// Setup a property group.
			group = p.name;

			Vector<String> hint_parts = p.hint_string.split(",");
			group_base = hint_parts[0];
			if (hint_parts.size() > 1) {
				section_depth = hint_parts[1].to_int();
			} else {
				section_depth = 0;
			}

			subgroup = "";
			subgroup_base = "";

			continue;

		} else if (p.usage & PROPERTY_USAGE_CATEGORY) {
			// Setup a property category.
			group = "";
			group_base = "";
			subgroup = "";
			subgroup_base = "";
			section_depth = 0;

			vbox_per_path.clear();
			editor_inspector_array_per_prefix.clear();

			// `hint_script` should contain a native class name or a script path.
			// Otherwise the category was probably added via `@export_category` or `_get_property_list()`.
			const bool is_custom_category = p.hint_string.is_empty();

			// Iterate over remaining properties. If no properties in category, skip the category.
			List<PropertyInfo>::Element *N = E_property->next();
			bool valid = true;
			while (N) {
				if (!N->get().name.begins_with("metadata/_") && N->get().usage & PROPERTY_USAGE_EDITOR &&
						(!filter.is_empty() || !restrict_to_basic || (N->get().usage & PROPERTY_USAGE_EDITOR_BASIC_SETTING))) {
					break;
				}
				// Treat custom categories as second-level ones. Do not skip a normal category if it is followed by a custom one.
				// Skip in the other 3 cases (normal -> normal, custom -> custom, custom -> normal).
				if ((N->get().usage & PROPERTY_USAGE_CATEGORY) && (is_custom_category || !N->get().hint_string.is_empty())) {
					valid = false;
					break;
				}
				N = N->next();
			}
			if (!valid) {
				continue; // Empty, ignore it.
			}

			String category_label;
			String category_tooltip;
			Ref<Texture> category_icon;

			// Do not add an icon, do not change the current class (`doc_name`) for custom categories.
			if (is_custom_category) {
				category_label = p.name;
				category_tooltip = p.name;
			} else {
				doc_name = p.name;
				category_label = p.name;

				// Use category's owner script to update some of its information.
				if (!EditorNode::get_editor_data().is_type_recognized(p.name) && ResourceLoader::exists(p.hint_string)) {
					Ref<Script> scr = ResourceLoader::load(p.hint_string, "Script");
					if (scr.is_valid()) {
						StringName script_name = EditorNode::get_editor_data().script_class_get_name(scr->get_path());

						// Update the docs reference and the label based on the script.
						Vector<DocData::ClassDoc> docs = scr->get_documentation();
						if (!docs.is_empty()) {
							// The documentation of a GDScript's main class is at the end of the array.
							// Hacky because this isn't necessarily always guaranteed.
							doc_name = docs[docs.size() - 1].name;
						}
						if (script_name != StringName()) {
							category_label = script_name;
						}

						// Find the icon corresponding to the script.
						if (script_name != StringName()) {
							category_icon = EditorNode::get_singleton()->get_class_icon(script_name);
						} else {
							category_icon = EditorNode::get_singleton()->get_object_icon(scr.ptr(), "Object");
						}
					}
				}

				if (category_icon.is_null() && !p.name.is_empty()) {
					category_icon = EditorNode::get_singleton()->get_class_icon(p.name);
				}

				if (use_doc_hints) {
					// `|` separators used in `EditorHelpBit`.
					category_tooltip = "class|" + doc_name + "|";
				}
			}

			if ((is_custom_category && !show_custom_categories) || (!is_custom_category && !show_standard_categories)) {
				continue;
			}

			// Hide the "MultiNodeEdit" category for MultiNodeEdit.
			if (Object::cast_to<MultiNodeEdit>(object) && p.name == "MultiNodeEdit") {
				continue;
			}

			// Create an EditorInspectorCategory and add it to the inspector.
			EditorInspectorCategory *category = memnew(EditorInspectorCategory);
			main_vbox->add_child(category);
			category_vbox = nullptr; // Reset.

			// Set the category info.
			category->label = category_label;
			category->set_tooltip_text(category_tooltip);
			category->icon = category_icon;
			if (!is_custom_category) {
				category->doc_class_name = doc_name;
			}

			// Add editors at the start of a category.
			for (Ref<EditorInspectorPlugin> &ped : valid_plugins) {
				ped->parse_category(object, p.name);
				_parse_added_editors(main_vbox, nullptr, ped);
			}

			continue;

		} else if (p.name.begins_with("metadata/_") || !(p.usage & PROPERTY_USAGE_EDITOR) || _is_property_disabled_by_feature_profile(p.name) ||
				(filter.is_empty() && restrict_to_basic && !(p.usage & PROPERTY_USAGE_EDITOR_BASIC_SETTING))) {
			// Ignore properties that are not supposed to be in the inspector.
			continue;
		}

		if (p.name == "script") {
			// Script should go into its own category.
			category_vbox = nullptr;
		}

		if (p.usage & PROPERTY_USAGE_HIGH_END_GFX && RS::get_singleton()->is_low_end()) {
			// Do not show this property in low end gfx.
			continue;
		}

		if (p.name == "script" && (hide_script || bool(object->call("_hide_script_from_inspector")))) {
			// Hide script variables from inspector if required.
			continue;
		}

		if (p.name.begins_with("metadata/") && bool(object->call("_hide_metadata_from_inspector"))) {
			// Hide metadata from inspector if required.
			continue;
		}

		// Get the path for property.
		String path = p.name;

		// First check if we have an array that fits the prefix.
		String array_prefix = "";
		int array_index = -1;
		for (KeyValue<String, EditorInspectorArray *> &E : editor_inspector_array_per_prefix) {
			if (p.name.begins_with(E.key) && E.key.length() > array_prefix.length()) {
				array_prefix = E.key;
			}
		}

		if (!array_prefix.is_empty()) {
			// If we have an array element, find the according index in array.
			String str = p.name.trim_prefix(array_prefix);
			int to_char_index = 0;
			while (to_char_index < str.length()) {
				if (!is_digit(str[to_char_index])) {
					break;
				}
				to_char_index++;
			}
			if (to_char_index > 0) {
				array_index = str.left(to_char_index).to_int();
			} else {
				array_prefix = "";
			}
		}

		if (!array_prefix.is_empty()) {
			path = path.trim_prefix(array_prefix);
			int char_index = path.find("/");
			if (char_index >= 0) {
				path = path.right(-char_index - 1);
			} else {
				path = vformat(TTR("Element %s"), array_index);
			}
		} else {
			// Check if we exit or not a subgroup. If there is a prefix, remove it from the property label string.
			if (!subgroup.is_empty() && !subgroup_base.is_empty()) {
				if (path.begins_with(subgroup_base)) {
					path = path.trim_prefix(subgroup_base);
				} else if (subgroup_base.begins_with(path)) {
					// Keep it, this is used pretty often.
				} else {
					subgroup = ""; // The prefix changed, we are no longer in the subgroup.
				}
			}

			// Check if we exit or not a group. If there is a prefix, remove it from the property label string.
			if (!group.is_empty() && !group_base.is_empty() && subgroup.is_empty()) {
				if (path.begins_with(group_base)) {
					path = path.trim_prefix(group_base);
				} else if (group_base.begins_with(path)) {
					// Keep it, this is used pretty often.
				} else {
					group = ""; // The prefix changed, we are no longer in the group.
					subgroup = "";
				}
			}

			// Add the group and subgroup to the path.
			if (!subgroup.is_empty()) {
				path = subgroup + "/" + path;
			}
			if (!group.is_empty()) {
				path = group + "/" + path;
			}
		}

		// Get the property label's string.
		String name_override = (path.contains("/")) ? path.substr(path.rfind("/") + 1) : path;
		String feature_tag;
		{
			const int dot = name_override.find(".");
			if (dot != -1) {
				feature_tag = name_override.substr(dot);
				name_override = name_override.substr(0, dot);
			}
		}

		// Don't localize script variables.
		EditorPropertyNameProcessor::Style name_style = property_name_style;
		if ((p.usage & PROPERTY_USAGE_SCRIPT_VARIABLE) && name_style == EditorPropertyNameProcessor::STYLE_LOCALIZED) {
			name_style = EditorPropertyNameProcessor::STYLE_CAPITALIZED;
		}
		const String property_label_string = EditorPropertyNameProcessor::get_singleton()->process_name(name_override, name_style, p.name, doc_name) + feature_tag;

		// Remove the property from the path.
		int idx = path.rfind("/");
		if (idx > -1) {
			path = path.left(idx);
		} else {
			path = "";
		}

		// Ignore properties that do not fit the filter.
		if (use_filter && !filter.is_empty()) {
			const String property_path = property_prefix + (path.is_empty() ? "" : path + "/") + name_override;
			if (!_property_path_matches(property_path, filter, property_name_style)) {
				continue;
			}
		}

		// Recreate the category vbox if it was reset.
		if (category_vbox == nullptr) {
			category_vbox = memnew(VBoxContainer);
			category_vbox->hide();
			main_vbox->add_child(category_vbox);
		}

		// Find the correct section/vbox to add the property editor to.
		VBoxContainer *root_vbox = array_prefix.is_empty() ? main_vbox : editor_inspector_array_per_prefix[array_prefix]->get_vbox(array_index);
		if (!root_vbox) {
			continue;
		}

		if (!vbox_per_path.has(root_vbox)) {
			vbox_per_path[root_vbox] = HashMap<String, VBoxContainer *>();
			vbox_per_path[root_vbox][""] = root_vbox;
		}

		VBoxContainer *current_vbox = root_vbox;
		String acc_path = "";
		int level = 1;

		Vector<String> components = path.split("/");
		for (int i = 0; i < components.size(); i++) {
			const String &component = components[i];
			acc_path += (i > 0) ? "/" + component : component;

			if (!vbox_per_path[root_vbox].has(acc_path)) {
				// If the section does not exists, create it.
				EditorInspectorSection *section = memnew(EditorInspectorSection);
				current_vbox->add_child(section);
				sections.push_back(section);

				String label;
				String tooltip;

				// Don't localize groups for script variables.
				EditorPropertyNameProcessor::Style section_name_style = property_name_style;
				if ((p.usage & PROPERTY_USAGE_SCRIPT_VARIABLE) && section_name_style == EditorPropertyNameProcessor::STYLE_LOCALIZED) {
					section_name_style = EditorPropertyNameProcessor::STYLE_CAPITALIZED;
				}

				// Only process group label if this is not the group or subgroup.
				if ((i == 0 && component == group) || (i == 1 && component == subgroup)) {
					if (section_name_style == EditorPropertyNameProcessor::STYLE_LOCALIZED) {
						label = EditorPropertyNameProcessor::get_singleton()->translate_group_name(component);
						tooltip = component;
					} else {
						label = component;
						tooltip = EditorPropertyNameProcessor::get_singleton()->translate_group_name(component);
					}
				} else {
					label = EditorPropertyNameProcessor::get_singleton()->process_name(component, section_name_style, p.name, doc_name);
					tooltip = EditorPropertyNameProcessor::get_singleton()->process_name(component, EditorPropertyNameProcessor::get_tooltip_style(section_name_style), p.name, doc_name);
				}

				Color c = sscolor;
				c.a /= level;
				section->setup(acc_path, label, object, c, use_folding, section_depth, level);
				section->set_tooltip_text(tooltip);

				// Add editors at the start of a group.
				for (Ref<EditorInspectorPlugin> &ped : valid_plugins) {
					ped->parse_group(object, path);
					_parse_added_editors(section->get_vbox(), section, ped);
				}

				vbox_per_path[root_vbox][acc_path] = section->get_vbox();
			}

			current_vbox = vbox_per_path[root_vbox][acc_path];
			level = (MIN(level + 1, 4));
		}

		// If we did not find a section to add the property to, add it to the category vbox instead (the category vbox handles margins correctly).
		if (current_vbox == main_vbox) {
			category_vbox->show();
			current_vbox = category_vbox;
		}

		// Check if the property is an array counter, if so create a dedicated array editor for the array.
		if (p.usage & PROPERTY_USAGE_ARRAY) {
			EditorInspectorArray *editor_inspector_array = nullptr;
			StringName array_element_prefix;
			Color c = sscolor;
			c.a /= level;

			Vector<String> class_name_components = String(p.class_name).split(",");

			int page_size = 5;
			bool movable = true;
			bool numbered = false;
			bool foldable = use_folding;
			String add_button_text = TTR("Add Element");
			String swap_method;
			for (int i = (p.type == Variant::NIL ? 1 : 2); i < class_name_components.size(); i++) {
				if (class_name_components[i].begins_with("page_size") && class_name_components[i].get_slice_count("=") == 2) {
					page_size = class_name_components[i].get_slice("=", 1).to_int();
				} else if (class_name_components[i].begins_with("add_button_text") && class_name_components[i].get_slice_count("=") == 2) {
					add_button_text = class_name_components[i].get_slice("=", 1).strip_edges();
				} else if (class_name_components[i] == "static") {
					movable = false;
				} else if (class_name_components[i] == "numbered") {
					numbered = true;
				} else if (class_name_components[i] == "unfoldable") {
					foldable = false;
				} else if (class_name_components[i].begins_with("swap_method") && class_name_components[i].get_slice_count("=") == 2) {
					swap_method = class_name_components[i].get_slice("=", 1).strip_edges();
				}
			}

			if (p.type == Variant::NIL) {
				// Setup the array to use a method to create/move/delete elements.
				array_element_prefix = class_name_components[0];
				editor_inspector_array = memnew(EditorInspectorArray(all_read_only));

				String array_label = path.contains("/") ? path.substr(path.rfind("/") + 1) : path;
				array_label = EditorPropertyNameProcessor::get_singleton()->process_name(property_label_string, property_name_style, p.name, doc_name);
				int page = per_array_page.has(array_element_prefix) ? per_array_page[array_element_prefix] : 0;
				editor_inspector_array->setup_with_move_element_function(object, array_label, array_element_prefix, page, c, use_folding);
				editor_inspector_array->connect("page_change_request", callable_mp(this, &DiffInspector::_page_change_request).bind(array_element_prefix));
			} else if (p.type == Variant::INT) {
				// Setup the array to use the count property and built-in functions to create/move/delete elements.
				if (class_name_components.size() >= 2) {
					array_element_prefix = class_name_components[1];
					editor_inspector_array = memnew(EditorInspectorArray(all_read_only));
					int page = per_array_page.has(array_element_prefix) ? per_array_page[array_element_prefix] : 0;

					editor_inspector_array->setup_with_count_property(object, class_name_components[0], p.name, array_element_prefix, page, c, foldable, movable, numbered, page_size, add_button_text, swap_method);
					editor_inspector_array->connect("page_change_request", callable_mp(this, &DiffInspector::_page_change_request).bind(array_element_prefix));
				}
			}

			if (editor_inspector_array) {
				current_vbox->add_child(editor_inspector_array);
				editor_inspector_array_per_prefix[array_element_prefix] = editor_inspector_array;
			}

			continue;
		}

		// Checkable and checked properties.
		bool checkable = false;
		bool checked = false;
		if (p.usage & PROPERTY_USAGE_CHECKABLE) {
			checkable = true;
			checked = p.usage & PROPERTY_USAGE_CHECKED;
		}

		bool property_read_only = (p.usage & PROPERTY_USAGE_READ_ONLY) || read_only;

		// Mark properties that would require an editor restart (mostly when editing editor settings).
		if (p.usage & PROPERTY_USAGE_RESTART_IF_CHANGED) {
			restart_request_props.insert(p.name);
		}

		String doc_path;
		String theme_item_name;
		StringName classname = doc_name;

		// Build the doc hint, to use as tooltip.
		if (use_doc_hints) {
			if (!object_class.is_empty()) {
				classname = object_class;
			} else if (Object::cast_to<MultiNodeEdit>(object)) {
				classname = Object::cast_to<MultiNodeEdit>(object)->get_edited_class_name();
			} else if (classname == "") {
				classname = object->get_class_name();
				Resource *res = Object::cast_to<Resource>(object);
				if (res && !res->get_script().is_null()) {
					// Grab the script of this resource to get the evaluated script class.
					Ref<Script> scr = res->get_script();
					if (scr.is_valid()) {
						Vector<DocData::ClassDoc> docs = scr->get_documentation();
						if (!docs.is_empty()) {
							// The documentation of a GDScript's main class is at the end of the array.
							// Hacky because this isn't necessarily always guaranteed.
							classname = docs[docs.size() - 1].name;
						}
					}
				}
			}

			StringName propname = property_prefix + p.name;
			bool found = false;

			// Small hack for theme_overrides. They are listed under Control, but come from another class.
			if (classname == "Control" && p.name.begins_with("theme_override_")) {
				classname = get_edited_object()->get_class();
			}

			// Search for the doc path in the cache.
			HashMap<StringName, HashMap<StringName, DocCacheInfo>>::Iterator E = doc_cache.find(classname);
			if (E) {
				HashMap<StringName, DocCacheInfo>::Iterator F = E->value.find(propname);
				if (F) {
					found = true;
					doc_path = F->value.doc_path;
					theme_item_name = F->value.theme_item_name;
				}
			}

			if (!found) {
				DocTools *dd = EditorHelp::get_doc_data();
				// Do not cache the doc path information of scripts.
				bool is_native_class = ClassDB::class_exists(classname);

				HashMap<String, DocData::ClassDoc>::ConstIterator F = dd->class_list.find(classname);
				while (F) {
					Vector<String> slices = propname.operator String().split("/");
					// Check if it's a theme item first.
					if (slices.size() == 2 && slices[0].begins_with("theme_override_")) {
						for (int i = 0; i < F->value.theme_properties.size(); i++) {
							String doc_path_current = "class_theme_item:" + F->value.name + ":" + F->value.theme_properties[i].name;
							if (F->value.theme_properties[i].name == slices[1]) {
								doc_path = doc_path_current;
								theme_item_name = F->value.theme_properties[i].name;
							}
						}
					} else {
						for (int i = 0; i < F->value.properties.size(); i++) {
							String doc_path_current = "class_property:" + F->value.name + ":" + F->value.properties[i].name;
							if (F->value.properties[i].name == propname.operator String()) {
								doc_path = doc_path_current;
							}
						}
					}

					if (is_native_class) {
						DocCacheInfo cache_info;
						cache_info.doc_path = doc_path;
						cache_info.theme_item_name = theme_item_name;
						doc_cache[classname][propname] = cache_info;
					}

					if (!doc_path.is_empty() || F->value.inherits.is_empty()) {
						break;
					}
					// Couldn't find the doc path in the class itself, try its super class.
					F = dd->class_list.find(F->value.inherits);
				}
			}
		}

		Vector<EditorInspectorPlugin::AddedEditor> editors;
		Vector<EditorInspectorPlugin::AddedEditor> late_editors;

		// Search for the inspector plugin that will handle the properties. Then add the correct property editor to it.
		for (Ref<EditorInspectorPlugin> &ped : valid_plugins) {
			bool exclusive = ped->parse_property(object, p.type, p.name, p.hint, p.hint_string, p.usage, wide_editors);

			for (const EditorInspectorPlugin::AddedEditor &F : ped->added_editors) {
				if (F.add_to_end) {
					late_editors.push_back(F);
				} else {
					editors.push_back(F);
				}
			}

			ped->added_editors.clear();

			if (exclusive) {
				break;
			}
		}

		editors.append_array(late_editors);

		for (int i = 0; i < editors.size(); i++) {
			EditorProperty *ep = Object::cast_to<EditorProperty>(editors[i].property_editor);
			const Vector<String> &properties = editors[i].properties;

			if (ep) {
				// Set all this before the control gets the ENTER_TREE notification.
				ep->object = object;

				if (properties.size()) {
					if (properties.size() == 1) {
						// Since it's one, associate:
						ep->property = properties[0];
						ep->property_path = property_prefix + properties[0];
						ep->property_usage = p.usage;
						// And set label?
					}
					if (!editors[i].label.is_empty()) {
						ep->set_label(editors[i].label);
					} else {
						// Use the existing one.
						ep->set_label(property_label_string);
					}

					for (int j = 0; j < properties.size(); j++) {
						String prop = properties[j];

						if (!editor_property_map.has(prop)) {
							editor_property_map[prop] = List<EditorProperty *>();
						}
						editor_property_map[prop].push_back(ep);
					}
				}

				Node *section_search = current_vbox->get_parent();
				while (section_search) {
					EditorInspectorSection *section = Object::cast_to<EditorInspectorSection>(section_search);
					if (section) {
						ep->connect("property_can_revert_changed", callable_mp(section, &EditorInspectorSection::property_can_revert_changed));
					}
					section_search = section_search->get_parent();
					if (Object::cast_to<EditorInspector>(section_search)) {
						// Skip sub-resource inspectors.
						break;
					}
				}

				ep->set_draw_warning(draw_warning);
				ep->set_use_folding(use_folding);
				ep->set_checkable(checkable);
				ep->set_checked(checked);
				ep->set_keying(keying);
				ep->set_read_only(property_read_only || all_read_only);
				ep->set_deletable(deletable_properties || p.name.begins_with("metadata/"));
			}

			current_vbox->add_child(editors[i].property_editor);

			if (ep) {
				// Eventually, set other properties/signals after the property editor got added to the tree.
				bool update_all = (p.usage & PROPERTY_USAGE_UPDATE_ALL_IF_MODIFIED);
				ep->connect("property_changed", callable_mp(this, &DiffInspector::_property_changed).bind(update_all));
				ep->connect("property_keyed", callable_mp(this, &DiffInspector::_property_keyed));
				ep->connect("property_deleted", callable_mp(this, &DiffInspector::_property_deleted), CONNECT_DEFERRED);
				ep->connect("property_keyed_with_value", callable_mp(this, &DiffInspector::_property_keyed_with_value));
				ep->connect("property_checked", callable_mp(this, &DiffInspector::_property_checked));
				ep->connect("property_pinned", callable_mp(this, &DiffInspector::_property_pinned));
				ep->connect("selected", callable_mp(this, &DiffInspector::_property_selected));
				ep->connect("multiple_properties_changed", callable_mp(this, &DiffInspector::_multiple_properties_changed));
				ep->connect("resource_selected", callable_mp(this, &DiffInspector::_resource_selected), CONNECT_DEFERRED);
				ep->connect("object_id_selected", callable_mp(this, &DiffInspector::_object_id_selected), CONNECT_DEFERRED);

				if (use_doc_hints) {
					// `|` separators used in `EditorHelpBit`.
					if (theme_item_name.is_empty()) {
						if (p.name.contains("shader_parameter/")) {
							ShaderMaterial *shader_material = Object::cast_to<ShaderMaterial>(object);
							if (shader_material) {
								ep->set_tooltip_text("property|" + shader_material->get_shader()->get_path() + "|" + property_prefix + p.name);
							}
						} else if (p.usage & PROPERTY_USAGE_INTERNAL) {
							ep->set_tooltip_text("internal_property|" + classname + "|" + property_prefix + p.name);
						} else {
							ep->set_tooltip_text("property|" + classname + "|" + property_prefix + p.name);
						}
					} else {
						ep->set_tooltip_text("theme_item|" + classname + "|" + theme_item_name);
					}
					ep->has_doc_tooltip = true;
				}

				ep->set_doc_path(doc_path);
				ep->set_internal(p.usage & PROPERTY_USAGE_INTERNAL);

				ep->update_property();
				ep->_update_pin_flags();
				ep->update_editor_property_status();
				ep->update_cache();

				if (current_selected && ep->property == current_selected) {
					ep->select(current_focusable);
				}
			}
		}
	}

	if (!hide_metadata && !object->call("_hide_metadata_from_inspector")) {
		// Add 4px of spacing between the "Add Metadata" button and the content above it.
		Control *spacer = memnew(Control);
		spacer->set_custom_minimum_size(Size2(0, 4) * EDSCALE);
		main_vbox->add_child(spacer);

		Button *add_md = DiffInspector::create_inspector_action_button(TTR("Add Metadata"));
		add_md->set_icon(get_editor_theme_icon(SNAME("Add")));
		add_md->connect(SceneStringName(pressed), callable_mp(this, &DiffInspector::_show_add_meta_dialog));
		main_vbox->add_child(add_md);
		if (all_read_only) {
			add_md->set_disabled(true);
		}
	}

	// Get the lists of to add at the end.
	for (Ref<EditorInspectorPlugin> &ped : valid_plugins) {
		ped->parse_end(object);
		_parse_added_editors(main_vbox, nullptr, ped);
	}

	if (is_main_editor_inspector()) {
		// Updating inspector might invalidate some editing owners.
		EditorNode::get_singleton()->hide_unused_editors();
	}
	set_follow_focus(true);
}

void DiffInspector::update_property(const String &p_prop) {
	if (!editor_property_map.has(p_prop)) {
		return;
	}

	for (EditorProperty *E : editor_property_map[p_prop]) {
		E->update_property();
		E->update_editor_property_status();
		E->update_cache();
	}
}

void DiffInspector::_clear(bool p_hide_plugins) {
	while (main_vbox->get_child_count()) {
		memdelete(main_vbox->get_child(0));
	}

	property_selected = StringName();
	property_focusable = -1;
	editor_property_map.clear();
	sections.clear();
	pending.clear();
	restart_request_props.clear();

	if (p_hide_plugins && is_main_editor_inspector()) {
		EditorNode::get_singleton()->hide_unused_editors(this);
	}
}

Object *DiffInspector::get_edited_object() {
	return object;
}

Object *DiffInspector::get_next_edited_object() {
	return next_object;
}

void DiffInspector::edit(Object *p_object) {
	if (object == p_object) {
		return;
	}

	next_object = p_object; // Some plugins need to know the next edited object when clearing the inspector.
	if (object) {
		if (likely(Variant(object).get_validated_object())) {
			object->disconnect(CoreStringName(property_list_changed), callable_mp(this, &DiffInspector::_changed_callback));
		}
		_clear();
	}
	per_array_page.clear();

	object = p_object;

	if (object) {
		update_scroll_request = 0; //reset
		if (scroll_cache.has(object->get_instance_id())) { //if exists, set something else
			update_scroll_request = scroll_cache[object->get_instance_id()]; //done this way because wait until full size is accommodated
		}
		object->connect(CoreStringName(property_list_changed), callable_mp(this, &DiffInspector::_changed_callback));
		update_tree();
	}

	// Keep it available until the end so it works with both main and sub inspectors.
	next_object = nullptr;

	emit_signal(SNAME("edited_object_changed"));
}

void DiffInspector::set_keying(bool p_active) {
	if (keying == p_active) {
		return;
	}
	keying = p_active;
	_keying_changed();
}

void DiffInspector::_keying_changed() {
	for (const KeyValue<StringName, List<EditorProperty *>> &F : editor_property_map) {
		for (EditorProperty *E : F.value) {
			if (E) {
				E->set_keying(keying);
			}
		}
	}
}

void DiffInspector::set_read_only(bool p_read_only) {
	if (p_read_only == read_only) {
		return;
	}
	read_only = p_read_only;
	update_tree();
}

EditorPropertyNameProcessor::Style DiffInspector::get_property_name_style() const {
	return property_name_style;
}

void DiffInspector::set_property_name_style(EditorPropertyNameProcessor::Style p_style) {
	if (property_name_style == p_style) {
		return;
	}
	property_name_style = p_style;
	update_tree();
}

void DiffInspector::set_use_settings_name_style(bool p_enable) {
	if (use_settings_name_style == p_enable) {
		return;
	}
	use_settings_name_style = p_enable;
	if (use_settings_name_style) {
		set_property_name_style(EditorPropertyNameProcessor::get_singleton()->get_settings_style());
	}
}

void DiffInspector::set_autoclear(bool p_enable) {
	autoclear = p_enable;
}

void DiffInspector::set_show_categories(bool p_show_standard, bool p_show_custom) {
	show_standard_categories = p_show_standard;
	show_custom_categories = p_show_custom;
	update_tree();
}

void DiffInspector::set_use_doc_hints(bool p_enable) {
	use_doc_hints = p_enable;
	update_tree();
}

void DiffInspector::set_hide_script(bool p_hide) {
	hide_script = p_hide;
	update_tree();
}

void DiffInspector::set_hide_metadata(bool p_hide) {
	hide_metadata = p_hide;
	update_tree();
}

void DiffInspector::set_use_filter(bool p_use) {
	use_filter = p_use;
	update_tree();
}

void DiffInspector::register_text_enter(Node *p_line_edit) {
	search_box = Object::cast_to<LineEdit>(p_line_edit);
	if (search_box) {
		search_box->connect(SceneStringName(text_changed), callable_mp(this, &DiffInspector::_filter_changed));
	}
}

void DiffInspector::_filter_changed(const String &p_text) {
	update_tree();
}

void DiffInspector::set_use_folding(bool p_use_folding, bool p_update_tree) {
	use_folding = p_use_folding;

	if (p_update_tree) {
		update_tree();
	}
}

bool DiffInspector::is_using_folding() {
	return use_folding;
}

void DiffInspector::collapse_all_folding() {
	for (EditorInspectorSection *E : sections) {
		E->fold();
	}

	for (const KeyValue<StringName, List<EditorProperty *>> &F : editor_property_map) {
		for (EditorProperty *E : F.value) {
			E->collapse_all_folding();
		}
	}
}

void DiffInspector::expand_all_folding() {
	for (EditorInspectorSection *E : sections) {
		E->unfold();
	}
	for (const KeyValue<StringName, List<EditorProperty *>> &F : editor_property_map) {
		for (EditorProperty *E : F.value) {
			E->expand_all_folding();
		}
	}
}

void DiffInspector::expand_revertable() {
	HashSet<EditorInspectorSection *> sections_to_unfold[2];
	for (EditorInspectorSection *E : sections) {
		if (E->has_revertable_properties()) {
			sections_to_unfold[0].insert(E);
		}
	}

	// Climb up the hierarchy doing double buffering with the sets.
	int a = 0;
	int b = 1;
	while (sections_to_unfold[a].size()) {
		for (EditorInspectorSection *E : sections_to_unfold[a]) {
			E->unfold();

			Node *n = E->get_parent();
			while (n) {
				if (Object::cast_to<EditorInspector>(n)) {
					break;
				}
				if (Object::cast_to<EditorInspectorSection>(n) && !sections_to_unfold[a].has((EditorInspectorSection *)n)) {
					sections_to_unfold[b].insert((EditorInspectorSection *)n);
				}
				n = n->get_parent();
			}
		}

		sections_to_unfold[a].clear();
		SWAP(a, b);
	}

	for (const KeyValue<StringName, List<EditorProperty *>> &F : editor_property_map) {
		for (EditorProperty *E : F.value) {
			E->expand_revertable();
		}
	}
}

void DiffInspector::set_scroll_offset(int p_offset) {
	set_v_scroll(p_offset);
}

int DiffInspector::get_scroll_offset() const {
	return get_v_scroll();
}

void DiffInspector::set_use_wide_editors(bool p_enable) {
	wide_editors = p_enable;
}

void DiffInspector::set_sub_inspector(bool p_enable) {
	sub_inspector = p_enable;
	if (!is_inside_tree()) {
		return;
	}
}

void DiffInspector::set_use_deletable_properties(bool p_enabled) {
	deletable_properties = p_enabled;
}

void DiffInspector::_page_change_request(int p_new_page, const StringName &p_array_prefix) {
	int prev_page = per_array_page.has(p_array_prefix) ? per_array_page[p_array_prefix] : 0;
	int new_page = MAX(0, p_new_page);
	if (new_page != prev_page) {
		per_array_page[p_array_prefix] = new_page;
		update_tree_pending = true;
	}
}

void DiffInspector::_edit_request_change(Object *p_object, const String &p_property) {
	if (object != p_object) { //may be undoing/redoing for a non edited object, so ignore
		return;
	}

	if (changing) {
		return;
	}

	if (p_property.is_empty()) {
		update_tree_pending = true;
	} else {
		pending.insert(p_property);
	}
}

void DiffInspector::_edit_set(const String &p_name, const Variant &p_value, bool p_refresh_all, const String &p_changed_field) {
	if (autoclear && editor_property_map.has(p_name)) {
		for (EditorProperty *E : editor_property_map[p_name]) {
			if (E->is_checkable()) {
				E->set_checked(true);
			}
		}
	}

	EditorUndoRedoManager *undo_redo = EditorUndoRedoManager::get_singleton();
	if (bool(object->call("_dont_undo_redo"))) {
		object->set(p_name, p_value);
		if (p_refresh_all) {
			_edit_request_change(object, "");
		} else {
			_edit_request_change(object, p_name);
		}

		emit_signal(_prop_edited, p_name);
	} else if (Object::cast_to<MultiNodeEdit>(object)) {
		Object::cast_to<MultiNodeEdit>(object)->set_property_field(p_name, p_value, p_changed_field);
		_edit_request_change(object, p_name);
		emit_signal(_prop_edited, p_name);
	} else {
		undo_redo->create_action(vformat(TTR("Set %s"), p_name), UndoRedo::MERGE_ENDS);
		undo_redo->add_do_property(object, p_name, p_value);
		bool valid = false;
		Variant value = object->get(p_name, &valid);
		if (valid) {
			undo_redo->add_undo_property(object, p_name, value);
		}

		List<StringName> linked_properties;
		ClassDB::get_linked_properties_info(object->get_class_name(), p_name, &linked_properties);

		for (const StringName &linked_prop : linked_properties) {
			valid = false;
			Variant undo_value = object->get(linked_prop, &valid);
			if (valid) {
				undo_redo->add_undo_property(object, linked_prop, undo_value);
			}
		}

		PackedStringArray linked_properties_dynamic = object->call("_get_linked_undo_properties", p_name, p_value);
		for (int i = 0; i < linked_properties_dynamic.size(); i++) {
			valid = false;
			Variant undo_value = object->get(linked_properties_dynamic[i], &valid);
			if (valid) {
				undo_redo->add_undo_property(object, linked_properties_dynamic[i], undo_value);
			}
		}

		Variant v_undo_redo = undo_redo;
		Variant v_object = object;
		Variant v_name = p_name;
		const Vector<Callable> &callbacks = EditorNode::get_editor_data().get_undo_redo_inspector_hook_callback();
		for (int i = 0; i < callbacks.size(); i++) {
			const Callable &callback = callbacks[i];

			const Variant *p_arguments[] = { &v_undo_redo, &v_object, &v_name, &p_value };
			Variant return_value;
			Callable::CallError call_error;

			callback.callp(p_arguments, 4, return_value, call_error);
			if (call_error.error != Callable::CallError::CALL_OK) {
				ERR_PRINT("Invalid UndoRedo callback.");
			}
		}

		if (p_refresh_all) {
			undo_redo->add_do_method(this, "_edit_request_change", object, "");
			undo_redo->add_undo_method(this, "_edit_request_change", object, "");
		} else {
			undo_redo->add_do_method(this, "_edit_request_change", object, p_name);
			undo_redo->add_undo_method(this, "_edit_request_change", object, p_name);
		}

		Resource *r = Object::cast_to<Resource>(object);
		if (r) {
			if (String(p_name) == "resource_local_to_scene") {
				bool prev = object->get(p_name);
				bool next = p_value;
				if (next) {
					undo_redo->add_do_method(r, "setup_local_to_scene");
				}
				if (prev) {
					undo_redo->add_undo_method(r, "setup_local_to_scene");
				}
			}
		}
		undo_redo->add_do_method(this, "emit_signal", _prop_edited, p_name);
		undo_redo->add_undo_method(this, "emit_signal", _prop_edited, p_name);
		undo_redo->commit_action();
	}

	if (editor_property_map.has(p_name)) {
		for (EditorProperty *E : editor_property_map[p_name]) {
			E->update_editor_property_status();
		}
	}
}

void DiffInspector::_property_changed(const String &p_path, const Variant &p_value, const String &p_name, bool p_changing, bool p_update_all) {
	// The "changing" variable must be true for properties that trigger events as typing occurs,
	// like "text_changed" signal. E.g. text property of Label, Button, RichTextLabel, etc.
	if (p_changing) {
		changing++;
	}

	_edit_set(p_path, p_value, p_update_all, p_name);

	if (p_changing) {
		changing--;
	}

	if (restart_request_props.has(p_path)) {
		emit_signal(SNAME("restart_requested"));
	}
}

void DiffInspector::_multiple_properties_changed(const Vector<String> &p_paths, const Array &p_values, bool p_changing) {
	ERR_FAIL_COND(p_paths.is_empty() || p_values.is_empty());
	ERR_FAIL_COND(p_paths.size() != p_values.size());
	String names;
	for (int i = 0; i < p_paths.size(); i++) {
		if (i > 0) {
			names += ",";
		}
		names += p_paths[i];
	}
	EditorUndoRedoManager *undo_redo = EditorUndoRedoManager::get_singleton();
	// TRANSLATORS: This is describing a change to multiple properties at once. The parameter is a list of property names.
	undo_redo->create_action(vformat(TTR("Set Multiple: %s"), names), UndoRedo::MERGE_ENDS);
	for (int i = 0; i < p_paths.size(); i++) {
		_edit_set(p_paths[i], p_values[i], false, "");
		if (restart_request_props.has(p_paths[i])) {
			emit_signal(SNAME("restart_requested"));
		}
	}
	if (p_changing) {
		changing++;
	}
	undo_redo->commit_action();
	if (p_changing) {
		changing--;
	}
}

void DiffInspector::_property_keyed(const String &p_path, bool p_advance) {
	if (!object) {
		return;
	}

	// The second parameter could be null, causing the event to fire with less arguments, so use the pointer call which preserves it.
	const Variant args[3] = { p_path, object->get(p_path), p_advance };
	const Variant *argp[3] = { &args[0], &args[1], &args[2] };
	emit_signalp(SNAME("property_keyed"), argp, 3);
}

void DiffInspector::_property_deleted(const String &p_path) {
	if (!object) {
		return;
	}

	if (p_path.begins_with("metadata/")) {
		String name = p_path.replace_first("metadata/", "");
		EditorUndoRedoManager *undo_redo = EditorUndoRedoManager::get_singleton();
		undo_redo->create_action(vformat(TTR("Remove metadata %s"), name));
		undo_redo->add_do_method(object, "remove_meta", name);
		undo_redo->add_undo_method(object, "set_meta", name, object->get_meta(name));
		undo_redo->commit_action();
	}

	emit_signal(SNAME("property_deleted"), p_path);
}

void DiffInspector::_property_keyed_with_value(const String &p_path, const Variant &p_value, bool p_advance) {
	if (!object) {
		return;
	}

	// The second parameter could be null, causing the event to fire with less arguments, so use the pointer call which preserves it.
	const Variant args[3] = { p_path, p_value, p_advance };
	const Variant *argp[3] = { &args[0], &args[1], &args[2] };
	emit_signalp(SNAME("property_keyed"), argp, 3);
}

void DiffInspector::_property_checked(const String &p_path, bool p_checked) {
	if (!object) {
		return;
	}

	//property checked
	if (autoclear) {
		if (!p_checked) {
			_edit_set(p_path, Variant(), false, "");
		} else {
			Variant to_create;
			List<PropertyInfo> pinfo;
			object->get_property_list(&pinfo);
			for (const PropertyInfo &E : pinfo) {
				if (E.name == p_path) {
					Callable::CallError ce;
					Variant::construct(E.type, to_create, nullptr, 0, ce);
					break;
				}
			}
			_edit_set(p_path, to_create, false, "");
		}

		if (editor_property_map.has(p_path)) {
			for (EditorProperty *E : editor_property_map[p_path]) {
				E->set_checked(p_checked);
				E->update_property();
				E->update_editor_property_status();
				E->update_cache();
			}
		}
	} else {
		emit_signal(SNAME("property_toggled"), p_path, p_checked);
	}
}

void DiffInspector::_property_pinned(const String &p_path, bool p_pinned) {
	if (!object) {
		return;
	}

	Node *node = Object::cast_to<Node>(object);
	ERR_FAIL_NULL(node);

	EditorUndoRedoManager *undo_redo = EditorUndoRedoManager::get_singleton();
	undo_redo->create_action(vformat(p_pinned ? TTR("Pinned %s") : TTR("Unpinned %s"), p_path));
	undo_redo->add_do_method(node, "_set_property_pinned", p_path, p_pinned);
	undo_redo->add_undo_method(node, "_set_property_pinned", p_path, !p_pinned);
	if (editor_property_map.has(p_path)) {
		for (List<EditorProperty *>::Element *E = editor_property_map[p_path].front(); E; E = E->next()) {
			undo_redo->add_do_method(E->get(), "_update_editor_property_status");
			undo_redo->add_undo_method(E->get(), "_update_editor_property_status");
		}
	}
	undo_redo->commit_action();
}

void DiffInspector::_property_selected(const String &p_path, int p_focusable) {
	property_selected = p_path;
	property_focusable = p_focusable;
	// Deselect the others.
	for (const KeyValue<StringName, List<EditorProperty *>> &F : editor_property_map) {
		if (F.key == property_selected) {
			continue;
		}
		for (EditorProperty *E : F.value) {
			if (E->is_selected()) {
				E->deselect();
			}
		}
	}

	emit_signal(SNAME("property_selected"), p_path);
}

void DiffInspector::_object_id_selected(const String &p_path, ObjectID p_id) {
	emit_signal(SNAME("object_id_selected"), p_id);
}

void DiffInspector::_resource_selected(const String &p_path, Ref<Resource> p_resource) {
	emit_signal(SNAME("resource_selected"), p_resource, p_path);
}

void DiffInspector::_node_removed(Node *p_node) {
	if (p_node == object) {
		edit(nullptr);
	}
}

void DiffInspector::_notification(int p_what) {
	switch (p_what) {
		case NOTIFICATION_THEME_CHANGED: {
			main_vbox->add_theme_constant_override("separation", get_theme_constant(SNAME("v_separation"), SNAME("EditorInspector")));
		} break;

		case NOTIFICATION_READY: {
			EditorFeatureProfileManager::get_singleton()->connect("current_feature_profile_changed", callable_mp(this, &DiffInspector::_feature_profile_changed));
			set_process(is_visible_in_tree());
			add_theme_style_override(SceneStringName(panel), get_theme_stylebox(SceneStringName(panel), SNAME("Tree")));
			if (!sub_inspector) {
				get_tree()->connect("node_removed", callable_mp(this, &DiffInspector::_node_removed));
			}
		} break;

		case NOTIFICATION_PREDELETE: {
			if (!sub_inspector && is_inside_tree()) {
				get_tree()->disconnect("node_removed", callable_mp(this, &DiffInspector::_node_removed));
			}
			edit(nullptr);
		} break;

		case NOTIFICATION_VISIBILITY_CHANGED: {
			set_process(is_visible_in_tree());
		} break;

		case NOTIFICATION_PROCESS: {
			if (update_scroll_request >= 0) {
				callable_mp((Range *)get_v_scroll_bar(), &Range::set_value).call_deferred(update_scroll_request);
				update_scroll_request = -1;
			}
			if (update_tree_pending) {
				refresh_countdown = float(EDITOR_GET("docks/property_editor/auto_refresh_interval"));
			} else if (refresh_countdown > 0) {
				refresh_countdown -= get_process_delta_time();
				if (refresh_countdown <= 0) {
					for (const KeyValue<StringName, List<EditorProperty *>> &F : editor_property_map) {
						for (EditorProperty *E : F.value) {
							if (E && !E->is_cache_valid()) {
								E->update_property();
								E->update_editor_property_status();
								E->update_cache();
							}
						}
					}
					refresh_countdown = float(EDITOR_GET("docks/property_editor/auto_refresh_interval"));
				}
			}

			changing++;

			if (update_tree_pending) {
				update_tree();
				update_tree_pending = false;
				pending.clear();

			} else {
				while (pending.size()) {
					StringName prop = *pending.begin();
					if (editor_property_map.has(prop)) {
						for (EditorProperty *E : editor_property_map[prop]) {
							E->update_property();
							E->update_editor_property_status();
							E->update_cache();
						}
					}
					pending.remove(pending.begin());
				}
			}

			changing--;
		} break;

		case EditorSettings::NOTIFICATION_EDITOR_SETTINGS_CHANGED: {
			bool needs_update = false;
			if (EditorThemeManager::is_generated_theme_outdated() && !sub_inspector) {
				add_theme_style_override(SceneStringName(panel), get_theme_stylebox(SceneStringName(panel), SNAME("Tree")));
			}

			if (use_settings_name_style && EditorSettings::get_singleton()->check_changed_settings_in_group("interface/editor/localize_settings")) {
				EditorPropertyNameProcessor::Style style = EditorPropertyNameProcessor::get_settings_style();
				if (property_name_style != style) {
					property_name_style = style;
					needs_update = true;
				}
			}

			if (EditorSettings::get_singleton()->check_changed_settings_in_group("interface/inspector")) {
				needs_update = true;
			}

			if (needs_update) {
				update_tree();
			}
		} break;
	}
}

void DiffInspector::_changed_callback() {
	//this is called when property change is notified via notify_property_list_changed()
	if (object != nullptr) {
		_edit_request_change(object, String());
	}
}

void DiffInspector::_vscroll_changed(double p_offset) {
	if (update_scroll_request >= 0) { //waiting, do nothing
		return;
	}

	if (object) {
		scroll_cache[object->get_instance_id()] = p_offset;
	}
}

void DiffInspector::set_property_prefix(const String &p_prefix) {
	property_prefix = p_prefix;
}

String DiffInspector::get_property_prefix() const {
	return property_prefix;
}

void DiffInspector::add_custom_property_description(const String &p_class, const String &p_property, const String &p_description) {
	const String key = vformat("property|%s|%s", p_class, p_property);
	custom_property_descriptions[key] = p_description;
}

String DiffInspector::get_custom_property_description(const String &p_property) const {
	HashMap<String, String>::ConstIterator E = custom_property_descriptions.find(p_property);
	if (E) {
		return E->value;
	}
	return "";
}

void DiffInspector::set_object_class(const String &p_class) {
	object_class = p_class;
}

String DiffInspector::get_object_class() const {
	return object_class;
}

void DiffInspector::_feature_profile_changed() {
	update_tree();
}

void DiffInspector::set_restrict_to_basic_settings(bool p_restrict) {
	restrict_to_basic = p_restrict;
	update_tree();
}

void DiffInspector::set_property_clipboard(const Variant &p_value) {
	property_clipboard = p_value;
}

Variant DiffInspector::get_property_clipboard() const {
	return property_clipboard;
}

void DiffInspector::_add_meta_confirm() {
	String name = add_meta_name->get_text();

	object->editor_set_section_unfold("metadata", true); // Ensure metadata is unfolded when adding a new metadata.

	Variant defval;
	Callable::CallError ce;
	Variant::construct(Variant::Type(add_meta_type->get_selected_id()), defval, nullptr, 0, ce);
	EditorUndoRedoManager *undo_redo = EditorUndoRedoManager::get_singleton();
	undo_redo->create_action(vformat(TTR("Add metadata %s"), name));
	undo_redo->add_do_method(object, "set_meta", name, defval);
	undo_redo->add_undo_method(object, "remove_meta", name);
	undo_redo->commit_action();
}

void DiffInspector::_check_meta_name() {
	const String meta_name = add_meta_name->get_text();

	if (meta_name.is_empty()) {
		validation_panel->set_message(EditorValidationPanel::MSG_ID_DEFAULT, TTR("Metadata name can't be empty."), EditorValidationPanel::MSG_ERROR);
	} else if (!meta_name.is_valid_identifier()) {
		validation_panel->set_message(EditorValidationPanel::MSG_ID_DEFAULT, TTR("Metadata name must be a valid identifier."), EditorValidationPanel::MSG_ERROR);
	} else if (object->has_meta(meta_name)) {
		validation_panel->set_message(EditorValidationPanel::MSG_ID_DEFAULT, vformat(TTR("Metadata with name \"%s\" already exists."), meta_name), EditorValidationPanel::MSG_ERROR);
	} else if (meta_name[0] == '_') {
		validation_panel->set_message(EditorValidationPanel::MSG_ID_DEFAULT, TTR("Names starting with _ are reserved for editor-only metadata."), EditorValidationPanel::MSG_ERROR);
	}
}

void DiffInspector::_show_add_meta_dialog() {
	if (!add_meta_dialog) {
		add_meta_dialog = memnew(ConfirmationDialog);

		VBoxContainer *vbc = memnew(VBoxContainer);
		add_meta_dialog->add_child(vbc);

		HBoxContainer *hbc = memnew(HBoxContainer);
		vbc->add_child(hbc);
		hbc->add_child(memnew(Label(TTR("Name:"))));

		add_meta_name = memnew(LineEdit);
		add_meta_name->set_custom_minimum_size(Size2(200 * EDSCALE, 1));
		hbc->add_child(add_meta_name);
		hbc->add_child(memnew(Label(TTR("Type:"))));

		add_meta_type = memnew(OptionButton);
		for (int i = 0; i < Variant::VARIANT_MAX; i++) {
			if (i == Variant::NIL || i == Variant::RID || i == Variant::CALLABLE || i == Variant::SIGNAL) {
				continue; //not editable by inspector.
			}
			String type = i == Variant::OBJECT ? String("Resource") : Variant::get_type_name(Variant::Type(i));

			add_meta_type->add_icon_item(get_editor_theme_icon(type), type, i);
		}
		hbc->add_child(add_meta_type);

		Control *spacing = memnew(Control);
		vbc->add_child(spacing);
		spacing->set_custom_minimum_size(Size2(0, 10 * EDSCALE));

		add_meta_dialog->set_ok_button_text(TTR("Add"));
		add_child(add_meta_dialog);
		add_meta_dialog->register_text_enter(add_meta_name);
		add_meta_dialog->connect(SceneStringName(confirmed), callable_mp(this, &DiffInspector::_add_meta_confirm));

		validation_panel = memnew(EditorValidationPanel);
		vbc->add_child(validation_panel);
		validation_panel->add_line(EditorValidationPanel::MSG_ID_DEFAULT, TTR("Metadata name is valid."));
		validation_panel->set_update_callback(callable_mp(this, &DiffInspector::_check_meta_name));
		validation_panel->set_accept_button(add_meta_dialog->get_ok_button());

		add_meta_name->connect(SceneStringName(text_changed), callable_mp(validation_panel, &EditorValidationPanel::update).unbind(1));
	}

	Node *node = Object::cast_to<Node>(object);
	if (node) {
		add_meta_dialog->set_title(vformat(TTR("Add Metadata Property for \"%s\""), node->get_name()));
	} else {
		// This should normally be reached when the object is derived from Resource.
		add_meta_dialog->set_title(vformat(TTR("Add Metadata Property for \"%s\""), object->get_class()));
	}

	add_meta_dialog->popup_centered();
	add_meta_name->grab_focus();
	add_meta_name->set_text("");
	validation_panel->update();
}

void DiffInspector::_bind_methods() {
	ClassDB::bind_method("_edit_request_change", &DiffInspector::_edit_request_change);
	ClassDB::bind_method("get_selected_path", &DiffInspector::get_selected_path);
	ClassDB::bind_method("get_edited_object", &DiffInspector::get_edited_object);

	ADD_SIGNAL(MethodInfo("property_selected", PropertyInfo(Variant::STRING, "property")));
	ADD_SIGNAL(MethodInfo("property_keyed", PropertyInfo(Variant::STRING, "property"), PropertyInfo(Variant::NIL, "value", PROPERTY_HINT_NONE, "", PROPERTY_USAGE_NIL_IS_VARIANT), PropertyInfo(Variant::BOOL, "advance")));
	ADD_SIGNAL(MethodInfo("property_deleted", PropertyInfo(Variant::STRING, "property")));
	ADD_SIGNAL(MethodInfo("resource_selected", PropertyInfo(Variant::OBJECT, "resource", PROPERTY_HINT_RESOURCE_TYPE, "Resource"), PropertyInfo(Variant::STRING, "path")));
	ADD_SIGNAL(MethodInfo("object_id_selected", PropertyInfo(Variant::INT, "id")));
	ADD_SIGNAL(MethodInfo("property_edited", PropertyInfo(Variant::STRING, "property")));
	ADD_SIGNAL(MethodInfo("property_toggled", PropertyInfo(Variant::STRING, "property"), PropertyInfo(Variant::BOOL, "checked")));
	ADD_SIGNAL(MethodInfo("edited_object_changed"));
	ADD_SIGNAL(MethodInfo("restart_requested"));
}

DiffInspector::EditorInspector() {
	object = nullptr;
	main_vbox = memnew(VBoxContainer);
	main_vbox->set_h_size_flags(SIZE_EXPAND_FILL);
	add_child(main_vbox);
	set_horizontal_scroll_mode(SCROLL_MODE_DISABLED);
	set_follow_focus(true);

	changing = 0;
	search_box = nullptr;
	_prop_edited = "property_edited";
	set_process(false);
	property_focusable = -1;
	property_clipboard = Variant();

	get_v_scroll_bar()->connect(SceneStringName(value_changed), callable_mp(this, &DiffInspector::_vscroll_changed));
	update_scroll_request = -1;
	if (EditorSettings::get_singleton()) {
		refresh_countdown = float(EDITOR_GET("docks/property_editor/auto_refresh_interval"));
	} else {
		//used when class is created by the docgen to dump default values of everything bindable, editorsettings may not be created
		refresh_countdown = 0.33;
	}

	ED_SHORTCUT("property_editor/copy_value", TTR("Copy Value"), KeyModifierMask::CMD_OR_CTRL | Key::C);
	ED_SHORTCUT("property_editor/paste_value", TTR("Paste Value"), KeyModifierMask::CMD_OR_CTRL | Key::V);
	ED_SHORTCUT("property_editor/copy_property_path", TTR("Copy Property Path"), KeyModifierMask::CMD_OR_CTRL | KeyModifierMask::SHIFT | Key::C);

	// `use_settings_name_style` is true by default, set the name style accordingly.
	set_property_name_style(EditorPropertyNameProcessor::get_singleton()->get_settings_style());
}

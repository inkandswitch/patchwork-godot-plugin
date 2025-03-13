/**************************************************************************/
/*  editor_inspector.h                                                    */
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

#ifndef EDITOR_INSPECTOR_H
#define EDITOR_INSPECTOR_H

#include "editor/editor_inspector.h"
#include "editor/editor_property_name_processor.h"
#include "scene/gui/box_container.h"
#include "scene/gui/scroll_container.h"

class AcceptDialog;
class Button;
class ConfirmationDialog;
class DiffInspector;
class EditorValidationPanel;
class LineEdit;
class MarginContainer;
class OptionButton;
class PanelContainer;
class PopupMenu;
class SpinBox;
class StyleBoxFlat;
class TextureRect;

class EditorPropertyRevert {
public:
	static Variant get_property_revert_value(Object *p_object, const StringName &p_property, bool *r_is_valid);
	static bool can_property_revert(Object *p_object, const StringName &p_property, const Variant *p_custom_current_value = nullptr);
};

class DiffInspectorCategory : public Control {
	GDCLASS(DiffInspectorCategory, Control);

	friend class DiffInspector;

	// Right-click context menu options.
	enum ClassMenuOption {
		MENU_OPEN_DOCS,
	};

	Ref<Texture2D> icon;
	String label;
	String doc_class_name;
	PopupMenu *menu = nullptr;

	void _handle_menu_option(int p_option);

protected:
	void _notification(int p_what);
	virtual void gui_input(const Ref<InputEvent> &p_event) override;

public:
	virtual Size2 get_minimum_size() const override;
	virtual Control *make_custom_tooltip(const String &p_text) const override;

	DiffInspectorCategory();
};

class DiffInspector : public ScrollContainer {
	GDCLASS(DiffInspector, ScrollContainer);

	enum {
		MAX_PLUGINS = 1024
	};
	static Ref<EditorInspectorPlugin> inspector_plugins[MAX_PLUGINS];
	static int inspector_plugin_count;

	VBoxContainer *main_vbox = nullptr;

	// Map used to cache the instantiated editors.
	HashMap<StringName, List<EditorProperty *>> editor_property_map;
	List<DiffInspectorSection *> sections;
	HashSet<StringName> pending;

	void _clear(bool p_hide_plugins = true);
	Object *object = nullptr;
	Object *next_object = nullptr;

	//

	LineEdit *search_box = nullptr;
	bool show_standard_categories = false;
	bool show_custom_categories = false;
	bool hide_script = true;
	bool hide_metadata = true;
	bool use_doc_hints = false;
	EditorPropertyNameProcessor::Style property_name_style = EditorPropertyNameProcessor::STYLE_CAPITALIZED;
	bool use_settings_name_style = true;
	bool use_filter = false;
	bool autoclear = false;
	bool use_folding = false;
	int changing;
	bool update_all_pending = false;
	bool read_only = false;
	bool keying = false;
	bool sub_inspector = false;
	bool wide_editors = false;
	bool deletable_properties = false;

	float refresh_countdown;
	bool update_tree_pending = false;
	StringName _prop_edited;
	StringName property_selected;
	int property_focusable;
	int update_scroll_request;

	struct DocCacheInfo {
		String doc_path;
		String theme_item_name;
	};

	HashMap<StringName, HashMap<StringName, DocCacheInfo>> doc_cache;
	HashSet<StringName> restart_request_props;
	HashMap<String, String> custom_property_descriptions;

	HashMap<ObjectID, int> scroll_cache;

	String property_prefix; // Used for sectioned inspector.
	String object_class;
	Variant property_clipboard;

	bool restrict_to_basic = false;

	void _edit_set(const String &p_name, const Variant &p_value, bool p_refresh_all, const String &p_changed_field);

	void _property_changed(const String &p_path, const Variant &p_value, const String &p_name = "", bool p_changing = false, bool p_update_all = false);
	void _multiple_properties_changed(const Vector<String> &p_paths, const Array &p_values, bool p_changing = false);
	void _property_keyed(const String &p_path, bool p_advance);
	void _property_keyed_with_value(const String &p_path, const Variant &p_value, bool p_advance);
	void _property_deleted(const String &p_path);
	void _property_checked(const String &p_path, bool p_checked);
	void _property_pinned(const String &p_path, bool p_pinned);
	bool _property_path_matches(const String &p_property_path, const String &p_filter, EditorPropertyNameProcessor::Style p_style);

	void _resource_selected(const String &p_path, Ref<Resource> p_resource);
	void _property_selected(const String &p_path, int p_focusable);
	void _object_id_selected(const String &p_path, ObjectID p_id);

	void _node_removed(Node *p_node);

	HashMap<StringName, int> per_array_page;
	void _page_change_request(int p_new_page, const StringName &p_array_prefix);

	void _changed_callback();
	void _edit_request_change(Object *p_object, const String &p_prop);

	void _keying_changed();

	void _filter_changed(const String &p_text);
	void _parse_added_editors(VBoxContainer *current_vbox, DiffInspectorSection *p_section, Ref<EditorInspectorPlugin> ped);

	void _vscroll_changed(double);

	void _feature_profile_changed();

	bool _is_property_disabled_by_feature_profile(const StringName &p_property);

	ConfirmationDialog *add_meta_dialog = nullptr;
	LineEdit *add_meta_name = nullptr;
	OptionButton *add_meta_type = nullptr;
	EditorValidationPanel *validation_panel = nullptr;

	void _add_meta_confirm();
	void _show_add_meta_dialog();
	void _check_meta_name();

protected:
	static void _bind_methods();
	void _notification(int p_what);

public:
	static void add_inspector_plugin(const Ref<EditorInspectorPlugin> &p_plugin);
	static void remove_inspector_plugin(const Ref<EditorInspectorPlugin> &p_plugin);
	static void cleanup_plugins();
	static Button *create_inspector_action_button(const String &p_text);

	static EditorProperty *instantiate_property_editor(Object *p_object, const Variant::Type p_type, const String &p_path, const PropertyHint p_hint, const String &p_hint_text, const uint32_t p_usage, const bool p_wide = false);

	bool is_main_editor_inspector() const;
	String get_selected_path() const;

	void update_tree();
	void update_property(const String &p_prop);
	void edit(Object *p_object);
	Object *get_edited_object();
	Object *get_next_edited_object();

	void set_keying(bool p_active);
	void set_read_only(bool p_read_only);

	EditorPropertyNameProcessor::Style get_property_name_style() const;
	void set_property_name_style(EditorPropertyNameProcessor::Style p_style);

	// If true, the inspector will update its property name style according to the current editor settings.
	void set_use_settings_name_style(bool p_enable);

	void set_autoclear(bool p_enable);

	void set_show_categories(bool p_show_standard, bool p_show_custom);
	void set_use_doc_hints(bool p_enable);
	void set_hide_script(bool p_hide);
	void set_hide_metadata(bool p_hide);

	void set_use_filter(bool p_use);
	void register_text_enter(Node *p_line_edit);

	void set_use_folding(bool p_use_folding, bool p_update_tree = true);
	bool is_using_folding();

	void collapse_all_folding();
	void expand_all_folding();
	void expand_revertable();

	void set_scroll_offset(int p_offset);
	int get_scroll_offset() const;

	void set_property_prefix(const String &p_prefix);
	String get_property_prefix() const;

	void add_custom_property_description(const String &p_class, const String &p_property, const String &p_description);
	String get_custom_property_description(const String &p_property) const;

	void set_object_class(const String &p_class);
	String get_object_class() const;

	void set_use_wide_editors(bool p_enable);
	void set_sub_inspector(bool p_enable);
	bool is_sub_inspector() const { return sub_inspector; }

	void set_use_deletable_properties(bool p_enabled);

	void set_restrict_to_basic_settings(bool p_restrict);
	void set_property_clipboard(const Variant &p_value);
	Variant get_property_clipboard() const;

	DiffInspector();
};

#endif // EDITOR_INSPECTOR_H

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

#include "core/error/error_macros.h"
#include "core/object/object.h"
#include "editor/diff_result.h"
#include "editor/editor_inspector.h"

Variant DiffInspector::get_property_revert_value(Object *p_object, const StringName &p_property) {
	bool r_is_valid;
	auto ret = EditorPropertyRevert::get_property_revert_value(p_object, p_property, &r_is_valid);
	if (r_is_valid) {
		return ret;
	}
	ERR_FAIL_V_MSG(Variant(), vformat("Failed to get revert value for property %s of object %s", p_property, p_object->get_class()));
}

bool DiffInspector::can_property_revert(Object *p_object, const StringName &p_property, bool has_current_value, Variant p_custom_current_value) {
	Variant *cur_value = nullptr;
	if (has_current_value) {
		cur_value = &p_custom_current_value;
	}
	return EditorPropertyRevert::can_property_revert(p_object, p_property, cur_value);
}

DiffInspectorProperty *DiffInspector::instantiate_property_editor(Object *p_object, const String &p_path, bool p_wide) {
	List<PropertyInfo> list;
	p_object->get_property_list(&list, false);
	PropertyInfo p_info;
	for (auto &E : list) {
		if (E.name == p_path) {
			p_info = E;
			break;
		}
	}
	auto ret = EditorInspector::instantiate_property_editor(p_object, p_info.type, p_path, p_info.hint, p_info.hint_string, p_info.usage, p_wide);
	if (!ret) {
		// iterate through all the special plugins and find one that parses
	}
	ERR_FAIL_COND_V_MSG(!ret, nullptr, vformat("Failed to instantiate property editor for %s", p_path));
	return static_cast<DiffInspectorProperty *>(ret);
}

void DiffInspector::_bind_methods() {
	ClassDB::bind_static_method("DiffInspector", D_METHOD("instantiate_property_editor", "object", "path", "wide"), &DiffInspector::instantiate_property_editor, DEFVAL(false));
	ClassDB::bind_static_method("DiffInspector", D_METHOD("get_property_revert_value", "object", "property"), &DiffInspector::get_property_revert_value);
	ClassDB::bind_static_method("DiffInspector", D_METHOD("can_property_revert", "object", "property", "has_current_value", "custom_current_value"), &DiffInspector::can_property_revert);
}

void DiffInspectorProperty::_bind_methods() {
	// just bind all the methods that are public/protected but are unbound
	// these are:
	// virtual void expand_all_folding();
	// virtual void collapse_all_folding();
	// virtual void expand_revertable();

	// virtual Variant get_drag_data(const Point2 &p_point) override;
	// virtual void update_cache();
	// virtual bool is_cache_valid() const;

	// void set_selectable(bool p_selectable);
	// bool is_selectable() const;

	// void set_name_split_ratio(float p_ratio);
	// float get_name_split_ratio() const;

	// void set_object_and_property(Object *p_object, const StringName &p_property);
	// virtual Control *make_custom_tooltip(const String &p_text) const override;

	// void set_draw_top_bg(bool p_draw) { draw_top_bg = p_draw; }

	// bool can_revert_to_default() const { return can_revert; }

	// void menu_option(int p_option);
	// 	void grab_focus(int p_focusable = -1);
	// void select(int p_focusable = -1);
	// void deselect();
	// bool is_selected() const;

	// void set_label_reference(Control *p_control);
	// void set_bottom_editor(Control *p_control);
	// void update_editor_property_status();
	ClassDB::bind_method(D_METHOD("update_property"), &DiffInspectorProperty::update_property);
	ClassDB::bind_method(D_METHOD("update_editor_property_status"), &DiffInspectorProperty::update_editor_property_status);

	ClassDB::bind_method(D_METHOD("grab_focus", "focusable"), &DiffInspectorProperty::grab_focus, DEFVAL(-1));
	ClassDB::bind_method(D_METHOD("select", "focusable"), &DiffInspectorProperty::select, DEFVAL(-1));
	ClassDB::bind_method(D_METHOD("deselect"), &DiffInspectorProperty::deselect);
	ClassDB::bind_method(D_METHOD("is_selected"), &DiffInspectorProperty::is_selected);
	ClassDB::bind_method(D_METHOD("set_label_reference", "control"), &DiffInspectorProperty::set_label_reference);
	ClassDB::bind_method(D_METHOD("set_bottom_editor", "control"), &DiffInspectorProperty::set_bottom_editor);
	ClassDB::bind_method(D_METHOD("set_read_only", "read_only"), &DiffInspectorProperty::set_read_only);
	ClassDB::bind_method(D_METHOD("is_read_only"), &DiffInspectorProperty::is_read_only);
	ClassDB::bind_method(D_METHOD("set_checkable", "checkable"), &DiffInspectorProperty::set_checkable);
	ClassDB::bind_method(D_METHOD("is_checkable"), &DiffInspectorProperty::is_checkable);
	ClassDB::bind_method(D_METHOD("set_checked", "checked"), &DiffInspectorProperty::set_checked);
	ClassDB::bind_method(D_METHOD("is_checked"), &DiffInspectorProperty::is_checked);
	ClassDB::bind_method(D_METHOD("set_draw_warning", "draw_warning"), &DiffInspectorProperty::set_draw_warning);
	ClassDB::bind_method(D_METHOD("is_draw_warning"), &DiffInspectorProperty::is_draw_warning);
	ClassDB::bind_method(D_METHOD("set_keying", "keying"), &DiffInspectorProperty::set_keying);
	ClassDB::bind_method(D_METHOD("is_keying"), &DiffInspectorProperty::is_keying);
	ClassDB::bind_method(D_METHOD("set_use_folding", "use_folding"), &DiffInspectorProperty::set_use_folding);
	ClassDB::bind_method(D_METHOD("is_using_folding"), &DiffInspectorProperty::is_using_folding);
	ClassDB::bind_method(D_METHOD("expand_all_folding"), &DiffInspectorProperty::expand_all_folding);
	ClassDB::bind_method(D_METHOD("collapse_all_folding"), &DiffInspectorProperty::collapse_all_folding);
	ClassDB::bind_method(D_METHOD("expand_revertable"), &DiffInspectorProperty::expand_revertable);
	ClassDB::bind_method(D_METHOD("get_drag_data", "point"), &DiffInspectorProperty::get_drag_data);
	ClassDB::bind_method(D_METHOD("update_cache"), &DiffInspectorProperty::update_cache);
	ClassDB::bind_method(D_METHOD("is_cache_valid"), &DiffInspectorProperty::is_cache_valid);
	ClassDB::bind_method(D_METHOD("set_selectable", "selectable"), &DiffInspectorProperty::set_selectable);
	ClassDB::bind_method(D_METHOD("is_selectable"), &DiffInspectorProperty::is_selectable);
	ClassDB::bind_method(D_METHOD("set_name_split_ratio", "ratio"), &DiffInspectorProperty::set_name_split_ratio);
	ClassDB::bind_method(D_METHOD("get_name_split_ratio"), &DiffInspectorProperty::get_name_split_ratio);
	ClassDB::bind_method(D_METHOD("set_object_and_property", "object", "property"), &DiffInspectorProperty::set_object_and_property);
	ClassDB::bind_method(D_METHOD("make_custom_tooltip", "text"), &DiffInspectorProperty::make_custom_tooltip);
	ClassDB::bind_method(D_METHOD("set_draw_top_bg", "draw"), &DiffInspectorProperty::set_draw_top_bg);
	ClassDB::bind_method(D_METHOD("can_revert_to_default"), &DiffInspectorProperty::can_revert_to_default);
	ClassDB::bind_method(D_METHOD("menu_option", "option"), &DiffInspectorProperty::menu_option);
}

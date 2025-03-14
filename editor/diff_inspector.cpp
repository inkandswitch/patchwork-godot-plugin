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

Control *DiffInspector::instantiate_property_editor(Object *p_object, const String &p_path, bool p_wide) {
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
	return ret;
}

void DiffInspector::_bind_methods() {
	ClassDB::bind_static_method("DiffInspector", D_METHOD("instantiate_property_editor", "object", "path", "wide"), &DiffInspector::instantiate_property_editor, DEFVAL(false));
	ClassDB::bind_static_method("DiffInspector", D_METHOD("get_property_revert_value", "object", "property"), &DiffInspector::get_property_revert_value);
	ClassDB::bind_static_method("DiffInspector", D_METHOD("can_property_revert", "object", "property", "has_current_value", "custom_current_value"), &DiffInspector::can_property_revert);
}

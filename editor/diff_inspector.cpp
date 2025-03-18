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

EditorProperty *DiffInspector::instantiate_property_editor(Object *p_object, Variant::Type p_type, const String &p_path, PropertyHint p_hint, const String &p_hint_text, uint32_t p_usage, bool p_wide) {
	return EditorInspector::instantiate_property_editor(p_object, p_type, p_path, p_hint, p_hint_text, p_usage, p_wide);
}

void DiffInspector::_bind_methods() {
	ClassDB::bind_static_method("DiffInspector", D_METHOD("instantiate_property_editor", "object", "type", "path", "hint", "hint_text", "usage", "wide"), &DiffInspector::instantiate_property_editor, DEFVAL(false));
	ClassDB::bind_static_method("DiffInspector", D_METHOD("get_property_revert_value", "object", "property"), &DiffInspector::get_property_revert_value);
	ClassDB::bind_static_method("DiffInspector", D_METHOD("can_property_revert", "object", "property", "has_current_value", "custom_current_value"), &DiffInspector::can_property_revert);
}

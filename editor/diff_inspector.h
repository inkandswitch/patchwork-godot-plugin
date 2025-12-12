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

#ifndef DIFF_INSPECTOR_H
#define DIFF_INSPECTOR_H

#include "core/version_generated.gen.h"
#if GODOT_VERSION_MAJOR == 4 && GODOT_VERSION_MINOR < 5
#include "editor/editor_inspector.h"
#else
#include "editor/inspector/editor_inspector.h"
#endif
#include "scene/gui/scroll_container.h"
#include "scene/resources/style_box_flat.h"
class EditorProperty;

class DiffInspector : public EditorInspector {
	GDCLASS(DiffInspector, EditorInspector);

protected:
	static void _bind_methods();

public:
	static Variant get_property_revert_value(Object *p_object, const StringName &p_property);
	static bool can_property_revert(Object *p_object, const StringName &p_property, bool has_current_value, Variant p_custom_current_value);
	static EditorProperty *instance_property_diff(Object *p_object, const String &p_path, bool p_wide = false);
};

#endif // DIFF_INSPECTOR_H

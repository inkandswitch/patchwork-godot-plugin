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

#include "editor/editor_inspector.h"
#include "scene/gui/scroll_container.h"
#include "scene/resources/style_box_flat.h"
class EditorProperty;

class DiffInspectorSection : public Container {
	GDCLASS(DiffInspectorSection, Container);

	String label;
	String section;
	bool vbox_added = false; // Optimization.
	Color bg_color;
	bool foldable = false;
	int indent_depth = 0;
	int level = 1;
	Point2 arrow_position;
	bool entered = false;

	Timer *dropping_unfold_timer = nullptr;
	bool dropping = false;
	bool dropping_for_unfold = false;

	HashSet<StringName> revertable_properties;

	bool unfolded = true;

	void _test_unfold();
	int _get_header_height() const;
	Ref<Texture2D> _get_arrow() const;
	String type = "changed";

protected:
	Object *object = nullptr;
	VBoxContainer *vbox = nullptr;

	void _notification(int p_what);
	static void _bind_methods();
	virtual void gui_input(const Ref<InputEvent> &p_event) override;

public:
	virtual Size2 get_minimum_size() const override;

	void setup(const String &p_section, const String &p_label, Object *p_object, const Color &p_bg_color, bool p_foldable, int p_indent_depth = 0, int p_level = 1);
	VBoxContainer *get_vbox();
	void unfold();
	void fold();
	void set_bg_color(const Color &p_bg_color);
	Color get_bg_color() const;
	bool has_revertable_properties() const;
	void property_can_revert_changed(const String &p_path, bool p_can_revert);

	void set_type(const String &p_type);
	String get_type() const;
	Object *get_object() const;
	void update_bg_color();

	bool is_folded() const;
	String get_section() const;

	String get_label() const;
	void set_label(const String &p_label);

	Rect2 get_header_rect() const;

	DiffInspectorSection();
	~DiffInspectorSection();
};

class DiffInspectorProperty : public EditorProperty {
	GDCLASS(DiffInspectorProperty, EditorProperty);

protected:
	static void _bind_methods();
};

class DiffInspector : public EditorInspector {
	GDCLASS(DiffInspector, EditorInspector);

protected:
	static void _bind_methods();

public:
	static Variant get_property_revert_value(Object *p_object, const StringName &p_property);
	static bool can_property_revert(Object *p_object, const StringName &p_property, bool has_current_value, Variant p_custom_current_value);
	static DiffInspectorProperty *instance_property_diff(Object *p_object, const String &p_path, bool p_wide = false);
};

#endif // DIFF_INSPECTOR_H

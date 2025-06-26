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
#include "editor/editor_inspector.h"
#include "editor/editor_string_names.h"
#include "editor/themes/editor_scale.h"
void DiffInspectorSection::_test_unfold() {
	if (!vbox_added) {
		add_child(vbox);
		move_child(vbox, 0);
		vbox_added = true;
	}
}

Ref<Texture2D> DiffInspectorSection::_get_arrow() {
	Ref<Texture2D> arrow;
	if (foldable) {
		if (object->editor_is_section_unfolded(section)) {
			arrow = get_theme_icon(SNAME("arrow"), SNAME("Tree"));
		} else {
			if (is_layout_rtl()) {
				arrow = get_theme_icon(SNAME("arrow_collapsed_mirrored"), SNAME("Tree"));
			} else {
				arrow = get_theme_icon(SNAME("arrow_collapsed"), SNAME("Tree"));
			}
		}
	}
	return arrow;
}

int DiffInspectorSection::_get_header_height() {
	Ref<Font> font = get_theme_font(SNAME("bold"), EditorStringName(EditorFonts));
	int font_size = get_theme_font_size(SNAME("bold_size"), EditorStringName(EditorFonts));

	int header_height = font->get_height(font_size);
	Ref<Texture2D> arrow = _get_arrow();
	if (arrow.is_valid()) {
		header_height = MAX(header_height, arrow->get_height());
	}
	header_height += get_theme_constant(SNAME("v_separation"), SNAME("Tree"));

	return header_height;
}

void DiffInspectorSection::update_bg_color() {
	if (type == "modified") {
		bg_color = get_theme_color(SNAME("prop_subsection_modified"), EditorStringName(Editor));
	} else if (type == "added") {
		bg_color = get_theme_color(SNAME("prop_subsection_added"), EditorStringName(Editor));
	} else if (type == "removed") {
		bg_color = get_theme_color(SNAME("prop_subsection_removed"), EditorStringName(Editor));
	} else {
		bg_color = get_theme_color(SNAME("prop_subsection"), EditorStringName(Editor));
	}
}

void DiffInspectorSection::set_type(const String &p_type) {
	type = p_type;
	update_bg_color();
}

String DiffInspectorSection::get_type() const {
	return type;
}

Object *DiffInspectorSection::get_object() const {
	return object;
}

void DiffInspectorSection::_notification(int p_what) {
	switch (p_what) {
		case NOTIFICATION_THEME_CHANGED: {
			update_minimum_size();
			update_bg_color();
			bg_color.a /= level;
		} break;

		case NOTIFICATION_SORT_CHILDREN: {
			if (!vbox_added) {
				return;
			}

			int inspector_margin = get_theme_constant(SNAME("inspector_margin"), EditorStringName(Editor));
			int section_indent_size = get_theme_constant(SNAME("indent_size"), SNAME("DiffInspectorSection"));
			if (indent_depth > 0 && section_indent_size > 0) {
				inspector_margin += indent_depth * section_indent_size;
			}
			Ref<StyleBoxFlat> section_indent_style = get_theme_stylebox(SNAME("indent_box"), SNAME("DiffInspectorSection"));
			if (indent_depth > 0 && section_indent_style.is_valid()) {
				inspector_margin += section_indent_style->get_margin(SIDE_LEFT) + section_indent_style->get_margin(SIDE_RIGHT);
			}

			Size2 size = get_size() - Vector2(inspector_margin, 0);
			int header_height = _get_header_height();
			Vector2 offset = Vector2(is_layout_rtl() ? 0 : inspector_margin, header_height);
			for (int i = 0; i < get_child_count(); i++) {
				Control *c = as_sortable_control(get_child(i));
				if (!c) {
					continue;
				}
				fit_child_in_rect(c, Rect2(offset, size));
			}
		} break;

		case NOTIFICATION_DRAW: {
			int section_indent = 0;
			int section_indent_size = get_theme_constant(SNAME("indent_size"), SNAME("DiffInspectorSection"));
			if (indent_depth > 0 && section_indent_size > 0) {
				section_indent = indent_depth * section_indent_size;
			}
			Ref<StyleBoxFlat> section_indent_style = get_theme_stylebox(SNAME("indent_box"), SNAME("DiffInspectorSection"));
			if (indent_depth > 0 && section_indent_style.is_valid()) {
				section_indent += section_indent_style->get_margin(SIDE_LEFT) + section_indent_style->get_margin(SIDE_RIGHT);
			}

			int header_width = get_size().width - section_indent;
			int header_offset_x = 0.0;
			bool rtl = is_layout_rtl();
			if (!rtl) {
				header_offset_x += section_indent;
			}

			// Draw header area.
			int header_height = _get_header_height();
			Rect2 header_rect = Rect2(Vector2(header_offset_x, 0.0), Vector2(header_width, header_height));
			Color c = bg_color;
			c.a *= 0.4;
			if (foldable && header_rect.has_point(get_local_mouse_position())) {
				c = c.lightened(Input::get_singleton()->is_mouse_button_pressed(MouseButton::LEFT) ? -0.05 : 0.2);
			}
			draw_rect(header_rect, c);

			// Draw header title, folding arrow and count of revertable properties.
			{
				int outer_margin = Math::round(2 * EDSCALE);
				int separation = get_theme_constant(SNAME("h_separation"), SNAME("DiffInspectorSection"));

				int margin_start = section_indent + outer_margin;
				int margin_end = outer_margin;

				// - Arrow.
				Ref<Texture2D> arrow = _get_arrow();
				arrow_position = Point2();
				if (arrow.is_valid()) {
					if (rtl) {
						arrow_position.x = get_size().width - (margin_start + arrow->get_width());
					} else {
						arrow_position.x = margin_start;
					}
					arrow_position.y = (header_height - arrow->get_height()) / 2;
					draw_texture(arrow, arrow_position);
					margin_start += arrow->get_width() + separation;
				}

				int available = get_size().width - (margin_start + margin_end);

				// - Count of revertable properties.
				String num_revertable_str;
				int num_revertable_width = 0;

				bool folded = foldable && !object->editor_is_section_unfolded(section);

				Ref<Font> font = get_theme_font(SNAME("bold"), EditorStringName(EditorFonts));
				int font_size = get_theme_font_size(SNAME("bold_size"), EditorStringName(EditorFonts));
				Color font_color = get_theme_color(SceneStringName(font_color), EditorStringName(Editor));

				if (folded && revertable_properties.size()) {
					int label_width = font->get_string_size(label, HORIZONTAL_ALIGNMENT_LEFT, available, font_size, TextServer::JUSTIFICATION_KASHIDA | TextServer::JUSTIFICATION_CONSTRAIN_ELLIPSIS).x;

					Ref<Font> light_font = get_theme_font(SNAME("main"), EditorStringName(EditorFonts));
					int light_font_size = get_theme_font_size(SNAME("main_size"), EditorStringName(EditorFonts));
					Color light_font_color = get_theme_color(SNAME("font_disabled_color"), EditorStringName(Editor));

					// Can we fit the long version of the revertable count text?
					num_revertable_str = vformat(TTRN("(%d change)", "(%d changes)", revertable_properties.size()), revertable_properties.size());
					num_revertable_width = light_font->get_string_size(num_revertable_str, HORIZONTAL_ALIGNMENT_LEFT, -1.0f, light_font_size, TextServer::JUSTIFICATION_NONE).x;
					if (label_width + outer_margin + num_revertable_width > available) {
						// We'll have to use the short version.
						num_revertable_str = vformat("(%d)", revertable_properties.size());
						num_revertable_width = light_font->get_string_size(num_revertable_str, HORIZONTAL_ALIGNMENT_LEFT, -1.0f, light_font_size, TextServer::JUSTIFICATION_NONE).x;
					}

					float text_offset_y = light_font->get_ascent(light_font_size) + (header_height - light_font->get_height(light_font_size)) / 2;
					Point2 text_offset = Point2(margin_end, text_offset_y).round();
					if (!rtl) {
						text_offset.x = get_size().width - (text_offset.x + num_revertable_width);
					}
					draw_string(light_font, text_offset, num_revertable_str, HORIZONTAL_ALIGNMENT_LEFT, -1.0f, light_font_size, light_font_color, TextServer::JUSTIFICATION_NONE);
					margin_end += num_revertable_width + outer_margin;
					available -= num_revertable_width + outer_margin;
				}

				// - Label.
				float text_offset_y = font->get_ascent(font_size) + (header_height - font->get_height(font_size)) / 2;
				Point2 text_offset = Point2(margin_start, text_offset_y).round();
				if (rtl) {
					text_offset.x = margin_end;
				}
				HorizontalAlignment text_align = rtl ? HORIZONTAL_ALIGNMENT_RIGHT : HORIZONTAL_ALIGNMENT_LEFT;
				draw_string(font, text_offset, label, text_align, available, font_size, font_color, TextServer::JUSTIFICATION_KASHIDA | TextServer::JUSTIFICATION_CONSTRAIN_ELLIPSIS);
			}

			// Draw dropping highlight.
			if (dropping && !vbox->is_visible_in_tree()) {
				Color accent_color = get_theme_color(SNAME("accent_color"), EditorStringName(Editor));
				draw_rect(Rect2(Point2(), get_size()), accent_color, false);
			}

			// Draw section indentation.
			if (section_indent_style.is_valid() && section_indent > 0) {
				Rect2 indent_rect = Rect2(Vector2(), Vector2(indent_depth * section_indent_size, get_size().height));
				if (rtl) {
					indent_rect.position.x = get_size().width - section_indent + section_indent_style->get_margin(SIDE_RIGHT);
				} else {
					indent_rect.position.x = section_indent_style->get_margin(SIDE_LEFT);
				}
				draw_style_box(section_indent_style, indent_rect);
			}
		} break;

		case NOTIFICATION_DRAG_BEGIN: {
			dropping_for_unfold = true;
		} break;

		case NOTIFICATION_DRAG_END: {
			dropping_for_unfold = false;
		} break;

		case NOTIFICATION_MOUSE_ENTER: {
			if (dropping || dropping_for_unfold) {
				dropping_unfold_timer->start();
			}
			queue_redraw();
		} break;

		case NOTIFICATION_MOUSE_EXIT: {
			if (dropping || dropping_for_unfold) {
				dropping_unfold_timer->stop();
			}
			queue_redraw();
		} break;
	}
}

Size2 DiffInspectorSection::get_minimum_size() const {
	Size2 ms;
	for (int i = 0; i < get_child_count(); i++) {
		Control *c = as_sortable_control(get_child(i));
		if (!c) {
			continue;
		}
		Size2 minsize = c->get_combined_minimum_size();
		ms = ms.max(minsize);
	}

	Ref<Font> font = get_theme_font(SceneStringName(font), SNAME("Tree"));
	int font_size = get_theme_font_size(SceneStringName(font_size), SNAME("Tree"));
	ms.height += font->get_height(font_size) + get_theme_constant(SNAME("v_separation"), SNAME("Tree"));
	ms.width += get_theme_constant(SNAME("inspector_margin"), EditorStringName(Editor));

	int section_indent_size = get_theme_constant(SNAME("indent_size"), SNAME("DiffInspectorSection"));
	if (indent_depth > 0 && section_indent_size > 0) {
		ms.width += indent_depth * section_indent_size;
	}
	Ref<StyleBoxFlat> section_indent_style = get_theme_stylebox(SNAME("indent_box"), SNAME("DiffInspectorSection"));
	if (indent_depth > 0 && section_indent_style.is_valid()) {
		ms.width += section_indent_style->get_margin(SIDE_LEFT) + section_indent_style->get_margin(SIDE_RIGHT);
	}

	return ms;
}

void DiffInspectorSection::setup(const String &p_section, const String &p_label, Object *p_object, const Color &p_bg_color, bool p_foldable, int p_indent_depth, int p_level) {
	section = p_section;
	label = p_label;
	object = p_object;
	bg_color = p_bg_color;
	foldable = p_foldable;
	indent_depth = p_indent_depth;
	level = p_level;

	if (!foldable && !vbox_added) {
		add_child(vbox);
		move_child(vbox, 0);
		vbox_added = true;
	}

	if (foldable) {
		_test_unfold();
		if (object->editor_is_section_unfolded(section)) {
			vbox->show();
		} else {
			vbox->hide();
		}
	}
}

void DiffInspectorSection::gui_input(const Ref<InputEvent> &p_event) {
	ERR_FAIL_COND(p_event.is_null());

	if (!foldable) {
		return;
	}

	Ref<InputEventMouseButton> mb = p_event;
	if (mb.is_valid() && mb->is_pressed() && mb->get_button_index() == MouseButton::LEFT) {
		// check the position of the arrow texture
		Ref<Texture2D> arrow = _get_arrow();
		if (arrow.is_valid()) {
			constexpr int FUDGE_FACTOR = 10;
			int bounding_width = arrow->get_width() + arrow_position.x + FUDGE_FACTOR;
			int bounding_height = get_size().y;
			Rect2 bounding_box = Rect2({ 0, 0 }, Vector2(bounding_width, bounding_height));
			if (bounding_box.has_point(mb->get_position())) {
				if (object->editor_is_section_unfolded(section)) {
					int header_height = _get_header_height();

					if (mb->get_position().y >= header_height) {
						return;
					}
				}

				accept_event();

				bool should_unfold = !object->editor_is_section_unfolded(section);
				if (should_unfold) {
					unfold();
				} else {
					fold();
				}
			} else {
				// otherwise, emit a signal
				emit_signal(SNAME("box_clicked"), section);
			}
		} else {
			// otherwise, emit a signal
			emit_signal(SNAME("box_clicked"), section);
		}
	} else if (mb.is_valid() && !mb->is_pressed()) {
		queue_redraw();
	}
}

VBoxContainer *DiffInspectorSection::get_vbox() {
	return vbox;
}

void DiffInspectorSection::unfold() {
	if (!foldable) {
		return;
	}

	_test_unfold();

	object->editor_set_section_unfold(section, true);
	vbox->show();
	queue_redraw();
}

void DiffInspectorSection::fold() {
	if (!foldable) {
		return;
	}

	if (!vbox_added) {
		return;
	}

	object->editor_set_section_unfold(section, false);
	vbox->hide();
	queue_redraw();
}

void DiffInspectorSection::set_bg_color(const Color &p_bg_color) {
	bg_color = p_bg_color;
	queue_redraw();
}

Color DiffInspectorSection::get_bg_color() const {
	return bg_color;
}

bool DiffInspectorSection::has_revertable_properties() const {
	return !revertable_properties.is_empty();
}

void DiffInspectorSection::property_can_revert_changed(const String &p_path, bool p_can_revert) {
	bool had_revertable_properties = has_revertable_properties();
	if (p_can_revert) {
		revertable_properties.insert(p_path);
	} else {
		revertable_properties.erase(p_path);
	}
	if (has_revertable_properties() != had_revertable_properties) {
		queue_redraw();
	}
}

void DiffInspectorSection::_bind_methods() {
	ClassDB::bind_method(D_METHOD("setup", "section", "label", "object", "bg_color", "foldable", "indent_depth", "level"), &DiffInspectorSection::setup, DEFVAL(0), DEFVAL(1));
	ClassDB::bind_method(D_METHOD("get_vbox"), &DiffInspectorSection::get_vbox);
	ClassDB::bind_method(D_METHOD("unfold"), &DiffInspectorSection::unfold);
	ClassDB::bind_method(D_METHOD("fold"), &DiffInspectorSection::fold);
	ClassDB::bind_method(D_METHOD("set_type", "type"), &DiffInspectorSection::set_type);
	ClassDB::bind_method(D_METHOD("get_type"), &DiffInspectorSection::get_type);
	ClassDB::bind_method(D_METHOD("get_object"), &DiffInspectorSection::get_object);
	// set/get bg color
	ClassDB::bind_method(D_METHOD("set_bg_color", "bg_color"), &DiffInspectorSection::set_bg_color);
	ClassDB::bind_method(D_METHOD("get_bg_color"), &DiffInspectorSection::get_bg_color);

	ADD_SIGNAL(MethodInfo("box_clicked", PropertyInfo(Variant::STRING, "section")));
}

DiffInspectorSection::DiffInspectorSection() {
	vbox = memnew(VBoxContainer);

	dropping_unfold_timer = memnew(Timer);
	dropping_unfold_timer->set_wait_time(0.6);
	dropping_unfold_timer->set_one_shot(true);
	add_child(dropping_unfold_timer);
	dropping_unfold_timer->connect("timeout", callable_mp(this, &DiffInspectorSection::unfold));
}

DiffInspectorSection::~DiffInspectorSection() {
	if (!vbox_added) {
		memdelete(vbox);
	}
}

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

DiffInspectorProperty *DiffInspector::instance_property_diff(Object *p_object, const String &p_path, bool p_wide) {
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
	ClassDB::bind_static_method("DiffInspector", D_METHOD("instance_property_diff", "object", "path", "wide"), &DiffInspector::instance_property_diff, DEFVAL(false));
	ClassDB::bind_static_method("DiffInspector", D_METHOD("get_property_revert_value", "object", "property"), &DiffInspector::get_property_revert_value);
	ClassDB::bind_static_method("DiffInspector", D_METHOD("can_property_revert", "object", "property", "has_current_value", "custom_current_value"), &DiffInspector::can_property_revert);
}

void DiffInspectorProperty::_bind_methods() {
	ClassDB::bind_method(D_METHOD("expand_all_folding"), &DiffInspectorProperty::expand_all_folding);
	ClassDB::bind_method(D_METHOD("collapse_all_folding"), &DiffInspectorProperty::collapse_all_folding);
	ClassDB::bind_method(D_METHOD("expand_revertable"), &DiffInspectorProperty::expand_revertable);
	ClassDB::bind_method(D_METHOD("get_drag_data", "point"), &DiffInspectorProperty::get_drag_data);
	ClassDB::bind_method(D_METHOD("update_cache"), &DiffInspectorProperty::update_cache);
	ClassDB::bind_method(D_METHOD("is_cache_valid"), &DiffInspectorProperty::is_cache_valid);
	ClassDB::bind_method(D_METHOD("make_custom_tooltip", "text"), &DiffInspectorProperty::make_custom_tooltip);
	ClassDB::bind_method(D_METHOD("set_draw_top_bg", "draw"), &DiffInspectorProperty::set_draw_top_bg);
	ClassDB::bind_method(D_METHOD("can_revert_to_default"), &DiffInspectorProperty::can_revert_to_default);
	ClassDB::bind_method(D_METHOD("menu_option", "option"), &DiffInspectorProperty::menu_option);
}

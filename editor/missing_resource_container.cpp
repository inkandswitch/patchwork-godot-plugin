/**************************************************************************/
/*  missing_resource.cpp                                                  */
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

#include "missing_resource_container.h"

inline String make_ident_compatible(String p_path) {
	// use is_ascii_identifier_char() to check if a char is valid for an identifier
	p_path = p_path.trim_prefix("res://");
	for (int i = 0; i < p_path.length(); i++) {
		if (!is_ascii_identifier_char(p_path[i])) {
			p_path.set(i, '_');
		}
	}
	return p_path;
}

inline String get_sanitized_res_path(const String &p_path) {
	return make_ident_compatible(p_path);
}

String get_prop_name(String file_path, String prop_name, String node_name = "") {
	if (node_name.is_empty()) {
		return get_sanitized_res_path(file_path) + "/" + prop_name;
	}
	return get_sanitized_res_path(file_path) + "/" + make_ident_compatible(node_name) + "/" + prop_name;
}

bool FakeInspectorResource::_set(const StringName &p_name, const Variant &p_value) {
	return false;
	// TODO?
	// String real_prop_name;
	//
	// auto ret = get_prop_dict_for_getset(p_name, TODO);
	// if (!ret.is_empty() && (is_recording_properties() || ret.has(p_name))) {
	// 	ret[p_name] = p_value;
	// 	return true;
	// }
	// return false;
}

Dictionary FakeInspectorResource::get_prop_dict_for_getset(String fq_prop_name, String &real_prop_name) const {
	for (const auto &E : file_diff_map) {
		if (fq_prop_name.begins_with(get_prop_name(E.key, ""))) {
			if (E.value.has("nodes")) {
				for (int i = 0; i < E.value["nodes"].operator Array().size(); i++) {
					Dictionary node_diff = ((Array)E.value["nodes"])[i];
					if (node_diff.has("props") && fq_prop_name.begins_with(get_prop_name(E.key, "", node_diff["path"]))) {
						return node_diff["props"];
					}
				}
			} else if (E.value.has("props")) {
				return E.value["props"];
			}
		}
	}
	return Dictionary();
}

bool FakeInspectorResource::_get(const StringName &p_name, Variant &r_ret) const {
	String name = p_name;
	bool old = name.ends_with("_old");
	bool new_ = name.ends_with("_new");
	if (old || new_) {
		name = name.substr(0, name.length() - 4);
	}
	Array arr;
	for (const auto &E : file_diff_map) {
		String prop_prefix = get_prop_name(E.key, "");
		if (name.begins_with(prop_prefix)) {
			if (E.value.has("nodes")) {
				for (int i = 0; i < E.value["nodes"].operator Array().size(); i++) {
					Dictionary node_diff = ((Array)E.value["nodes"])[i];
					prop_prefix = get_prop_name(E.key, "", node_diff["path"]);
					if (node_diff.has("props") && name.begins_with(prop_prefix)) {
						arr = ((Dictionary)node_diff["props"])[name.trim_prefix(prop_prefix)];
						break;
					}
				}
			} else if (E.value.has("props")) {
				arr = ((Dictionary)E.value["props"])[name.trim_prefix(prop_prefix)];
				break;
			}
		}
	}
	if (arr.size() > 0) {
		if ((old || new_) && arr.size() == 2) {
			auto old_var = arr[0];
			auto new_var = arr[1];
			r_ret = old ? old_var : new_var;
		} else {
			r_ret = arr;
		}
		return true;
	}
	return false;
}

void FakeInspectorResource::add_props_to_list(const String &name, const Dictionary &props, List<PropertyInfo> *p_list, const String &node_name) const {
	for (const auto &K : props.keys()) {
		auto prop_name = get_prop_name(name, (String)K, node_name);
		Array arr = props[K];
		Variant thing = arr.is_empty() ? Variant() : arr[0];
		p_list->push_back(PropertyInfo(thing.get_type(), prop_name + "_old"));
		p_list->push_back(PropertyInfo(thing.get_type(), prop_name + "_new"));
	}
}

inline PropertyInfo make_category(String path) {
	return PropertyInfo(Variant::NIL, path, PROPERTY_HINT_NONE, path, PROPERTY_USAGE_CATEGORY);
}

bool FakeInspectorResource::add_property_list_from_diff(String path, Dictionary diff, List<PropertyInfo> *p_list) const {
	// push back a divider
	String fqn = get_sanitized_res_path(path);
	String type = diff["type"];
	if (type == "deleted") {
		p_list->push_back({ Variant::NIL, fqn + "/(deleted)" });
		return true;
	}
	if (type == "added") {
		p_list->push_back({ Variant::INT, fqn + "/(added)" });
		return true;
	}
	if (type == "type_changed") {
		p_list->push_back(PropertyInfo(Variant::STRING, fqn + "/old_type"));
		p_list->push_back(PropertyInfo(Variant::STRING, fqn + "/new_type"));
		return true;
	}
	// p_list->push_back(make_category(path));
	if (type == "resource_changed") {
		add_props_to_list(path, diff["props"], p_list);
		return true;
	}
	if (type == "scene_changed") {
		Array node_diffs = diff["nodes"];
		for (int i = 0; i < node_diffs.size(); i++) {
			Dictionary node_diff = node_diffs[i];
			String node_path = node_diff["path"];
			String fqn_node = fqn + "/" + node_path;

			String change_type = node_diff["type"];
			// p_list->push_back(make_category(node_path));
			if (change_type == "node_deleted") {
				p_list->push_back({ Variant::NIL, fqn_node + "/(deleted)" });
			} else if (change_type == "node_added") {
				p_list->push_back({ Variant::NIL, fqn_node + "/(added)" });
			} else if (change_type == "node_changed") {
				Dictionary node_props = node_diff["props"];
				add_props_to_list(path, node_props, p_list, node_path);
			}
		}
		return true;
	}
	return false;
}

void FakeInspectorResource::_get_property_list(List<PropertyInfo> *p_list) const {
	// remove the Resource properties
	p_list->clear();
	for (const auto &E : file_diff_map) {
		if (!add_property_list_from_diff(E.key, E.value, p_list)) {
			ERR_CONTINUE("Invalid diff type!");
		}
	}
}

void FakeInspectorResource::add_file_diff(const String &file, const Dictionary &props) {
	file_diff_map.insert(file, props);
}

void FakeInspectorResource::add_diff(const Dictionary &props) {
	if (props.has("files")) {
		Array files = props["files"];
		for (int i = 0; i < files.size(); i++) {
			add_file_diff(files[i].operator Dictionary()["path"], files[i]);
		}
	} else if (props.has("path")) {
		add_file_diff(props["path"], props);
	}
}

void FakeInspectorResource::set_original_class(const String &p_class) {
	original_class = p_class;
}

String FakeInspectorResource::get_original_class() const {
	return original_class;
}

void FakeInspectorResource::set_recording_properties(bool p_enable) {
	recording_properties = p_enable;
}

bool FakeInspectorResource::is_recording_properties() const {
	return recording_properties;
}

void FakeInspectorResource::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_original_class", "name"), &FakeInspectorResource::set_original_class);
	ClassDB::bind_method(D_METHOD("get_original_class"), &FakeInspectorResource::get_original_class);

	ClassDB::bind_method(D_METHOD("set_recording_properties", "enable"), &FakeInspectorResource::set_recording_properties);
	ClassDB::bind_method(D_METHOD("is_recording_properties"), &FakeInspectorResource::is_recording_properties);

	ClassDB::bind_method(D_METHOD("add_file_diff", "file", "props"), &FakeInspectorResource::add_file_diff);
	ClassDB::bind_method(D_METHOD("add_diff", "props"), &FakeInspectorResource::add_diff);

	// Expose, but not save.
	ADD_PROPERTY(PropertyInfo(Variant::STRING, "original_class", PROPERTY_HINT_NONE, "", PROPERTY_USAGE_NONE), "set_original_class", "get_original_class");
	ADD_PROPERTY(PropertyInfo(Variant::BOOL, "recording_properties", PROPERTY_HINT_NONE, "", PROPERTY_USAGE_NONE), "set_recording_properties", "is_recording_properties");
}

FakeInspectorResource::FakeInspectorResource() {
}

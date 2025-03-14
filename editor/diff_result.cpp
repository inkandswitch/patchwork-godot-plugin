#include "diff_result.h"

void DiffResult::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_file_diff", "path", "diff"), &DiffResult::set_file_diff);
	ClassDB::bind_method(D_METHOD("get_file_diff", "path"), &DiffResult::get_file_diff);
	ClassDB::bind_method(D_METHOD("get_file_diffs"), &DiffResult::get_file_diffs);
}

void DiffResult::set_file_diff(const String &p_path, const Ref<FileDiffResult> &p_diff) {
	file_diffs[(Variant)p_path] = p_diff;
}

Ref<FileDiffResult> DiffResult::get_file_diff(const String &p_path) const {
	if (file_diffs.has((Variant)p_path)) {
		return file_diffs[(Variant)p_path];
	}
	return Ref<FileDiffResult>();
}

Dictionary DiffResult::get_file_diffs() const {
	return file_diffs;
}

void FileDiffResult::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_type", "type"), &FileDiffResult::set_type);
	ClassDB::bind_method(D_METHOD("get_type"), &FileDiffResult::get_type);
	ClassDB::bind_method(D_METHOD("set_res_old", "res"), &FileDiffResult::set_res_old);
	ClassDB::bind_method(D_METHOD("get_res_old"), &FileDiffResult::get_res_old);
	ClassDB::bind_method(D_METHOD("set_res_new", "res"), &FileDiffResult::set_res_new);
	ClassDB::bind_method(D_METHOD("get_res_new"), &FileDiffResult::get_res_new);
	ClassDB::bind_method(D_METHOD("set_props", "props"), &FileDiffResult::set_props);
	ClassDB::bind_method(D_METHOD("get_props"), &FileDiffResult::get_props);
	ClassDB::bind_method(D_METHOD("set_node_diffs", "diffs"), &FileDiffResult::set_node_diffs);
	ClassDB::bind_method(D_METHOD("get_node_diffs"), &FileDiffResult::get_node_diffs);

	ADD_PROPERTY(PropertyInfo(Variant::STRING, "type"), "set_type", "get_type");
	ADD_PROPERTY(PropertyInfo(Variant::OBJECT, "res_old", PROPERTY_HINT_RESOURCE_TYPE, "Resource"), "set_res_old", "get_res_old");
	ADD_PROPERTY(PropertyInfo(Variant::OBJECT, "res_new", PROPERTY_HINT_RESOURCE_TYPE, "Resource"), "set_res_new", "get_res_new");
	ADD_PROPERTY(PropertyInfo(Variant::DICTIONARY, "props"), "set_props", "get_props");
	ADD_PROPERTY(PropertyInfo(Variant::DICTIONARY, "node_diffs"), "set_node_diffs", "get_node_diffs");
}

void FileDiffResult::set_type(const String &p_type) {
	type = p_type;
}

String FileDiffResult::get_type() const {
	return type;
}

void FileDiffResult::set_res_old(const Ref<Resource> &p_res) {
	res_old = p_res;
}

Ref<Resource> FileDiffResult::get_res_old() const {
	return res_old;
}

void FileDiffResult::set_res_new(const Ref<Resource> &p_res) {
	res_new = p_res;
}

Ref<Resource> FileDiffResult::get_res_new() const {
	return res_new;
}

void FileDiffResult::set_props(const Ref<ObjectDiffResult> &p_props) {
	props = p_props;
}

Ref<ObjectDiffResult> FileDiffResult::get_props() const {
	return props;
}

void FileDiffResult::set_node_diffs(const Dictionary &p_diffs) {
	node_diffs = p_diffs;
}

Dictionary FileDiffResult::get_node_diffs() const {
	return node_diffs;
}

void ObjectDiffResult::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_property_diff", "name", "diff"), &ObjectDiffResult::set_property_diff);
	ClassDB::bind_method(D_METHOD("get_property_diff", "name"), &ObjectDiffResult::get_property_diff);
	ClassDB::bind_method(D_METHOD("get_property_diffs"), &ObjectDiffResult::get_property_diffs);
}

void ObjectDiffResult::set_property_diff(const Ref<PropertyDiffResult> &p_diff) {
	property_diffs[(Variant)p_diff->get_name()] = p_diff;
}

Ref<PropertyDiffResult> ObjectDiffResult::get_property_diff(const String &p_name) const {
	if (property_diffs.has((Variant)p_name)) {
		return property_diffs[(Variant)p_name];
	}
	return Ref<PropertyDiffResult>();
}

Dictionary ObjectDiffResult::get_property_diffs() const {
	return property_diffs;
}

void NodeDiffResult::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_path", "path"), &NodeDiffResult::set_path);
	ClassDB::bind_method(D_METHOD("get_path"), &NodeDiffResult::get_path);
	ClassDB::bind_method(D_METHOD("set_type", "type"), &NodeDiffResult::set_type);
	ClassDB::bind_method(D_METHOD("get_type"), &NodeDiffResult::get_type);
	ClassDB::bind_method(D_METHOD("set_props", "props"), &NodeDiffResult::set_props);
	ClassDB::bind_method(D_METHOD("get_props"), &NodeDiffResult::get_props);
}

void NodeDiffResult::set_path(const NodePath &p_path) {
	path = p_path;
}

NodePath NodeDiffResult::get_path() const {
	return path;
}

void NodeDiffResult::set_type(const String &p_type) {
	type = p_type;
}

String NodeDiffResult::get_type() const {
	return type;
}

void NodeDiffResult::set_props(const Ref<ObjectDiffResult> &p_props) {
	props = p_props;
}

Ref<ObjectDiffResult> NodeDiffResult::get_props() const {
	return props;
}

void PropertyDiffResult::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_name", "name"), &PropertyDiffResult::set_name);
	ClassDB::bind_method(D_METHOD("get_name"), &PropertyDiffResult::get_name);
	ClassDB::bind_method(D_METHOD("set_change_type", "change_type"), &PropertyDiffResult::set_change_type);
	ClassDB::bind_method(D_METHOD("get_change_type"), &PropertyDiffResult::get_change_type);
	ClassDB::bind_method(D_METHOD("set_old_value", "old_value"), &PropertyDiffResult::set_old_value);
	ClassDB::bind_method(D_METHOD("get_old_value"), &PropertyDiffResult::get_old_value);
	ClassDB::bind_method(D_METHOD("set_new_value", "new_value"), &PropertyDiffResult::set_new_value);
	ClassDB::bind_method(D_METHOD("get_new_value"), &PropertyDiffResult::get_new_value);
	ClassDB::bind_method(D_METHOD("set_old_object", "old_object"), &PropertyDiffResult::set_old_object);
	ClassDB::bind_method(D_METHOD("get_old_object"), &PropertyDiffResult::get_old_object);
	ClassDB::bind_method(D_METHOD("set_new_object", "new_object"), &PropertyDiffResult::set_new_object);
	ClassDB::bind_method(D_METHOD("get_new_object"), &PropertyDiffResult::get_new_object);
}

PropertyDiffResult::PropertyDiffResult() {
}

void PropertyDiffResult::set_name(const String &p_name) {
	name = p_name;
}

String PropertyDiffResult::get_name() const {
	return name;
}

void PropertyDiffResult::set_change_type(const String &p_change_type) {
	change_type = p_change_type;
}

String PropertyDiffResult::get_change_type() const {
	return change_type;
}

void PropertyDiffResult::set_old_value(const Variant &p_old_value) {
	old_value = p_old_value;
}

Variant PropertyDiffResult::get_old_value() const {
	return old_value;
}

void PropertyDiffResult::set_new_value(const Variant &p_new_value) {
	new_value = p_new_value;
}

Variant PropertyDiffResult::get_new_value() const {
	return new_value;
}

void PropertyDiffResult::set_old_object(Object *p_old_object) {
	old_object = p_old_object;
}

Object *PropertyDiffResult::get_old_object() const {
	return old_object;
}

void PropertyDiffResult::set_new_object(Object *p_new_object) {
	new_object = p_new_object;
}

Object *PropertyDiffResult::get_new_object() const {
	return new_object;
}

PropertyDiffResult::PropertyDiffResult(const String &p_name, const String &p_change_type, const Variant &p_old_value, const Variant &p_new_value, Object *p_old_object, Object *p_new_object) {
	name = p_name;
	change_type = p_change_type;
	old_value = p_old_value;
	new_value = p_new_value;
	old_object = p_old_object;
	new_object = p_new_object;
}
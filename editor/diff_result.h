#ifndef DIFF_RESULT_H
#define DIFF_RESULT_H

#include "core/object/ref_counted.h"
#include "core/variant/array.h"
#include "core/variant/dictionary.h"
#include "core/variant/variant.h"
#include "scene/main/node.h"
#include "scene/resources/packed_scene.h"

class Resource;
class FileDiffResult;
class ObjectDiffResult;
class NodeDiffResult;
class PropertyDiffResult;
class DiffResult : public RefCounted {
	GDCLASS(DiffResult, RefCounted)

private:
	Dictionary file_diffs;

protected:
	static void _bind_methods();

public:
	void set_file_diff(const String &p_path, const Ref<FileDiffResult> &p_diff);
	Ref<FileDiffResult> get_file_diff(const String &p_path) const;
	Dictionary get_file_diffs() const;
};

class FileDiffResult : public RefCounted {
	GDCLASS(FileDiffResult, RefCounted)

private:
	String type;
	Ref<Resource> res_old;
	Ref<Resource> res_new;
	Ref<ObjectDiffResult> props;
	Dictionary node_diffs;

protected:
	static void _bind_methods();

public:
	void set_type(const String &p_type);
	String get_type() const;
	void set_res_old(const Ref<Resource> &p_res);
	Ref<Resource> get_res_old() const;
	void set_res_new(const Ref<Resource> &p_res);
	Ref<Resource> get_res_new() const;
	void set_props(const Ref<ObjectDiffResult> &p_props);
	Ref<ObjectDiffResult> get_props() const;
	void set_node_diffs(const Dictionary &p_diffs);
	Dictionary get_node_diffs() const;
};

class ObjectDiffResult : public RefCounted {
	GDCLASS(ObjectDiffResult, RefCounted)

	Object *old_object;
	Object *new_object;
	Dictionary property_diffs;

protected:
	static void _bind_methods();

public:
	void set_old_object(Object *p_old_object);
	Object *get_old_object() const;
	void set_new_object(Object *p_new_object);
	Object *get_new_object() const;
	void set_property_diffs(const Dictionary &p_diffs);
	Dictionary get_property_diffs() const;
	void set_property_diff(const Ref<PropertyDiffResult> &p_diff);
	Ref<PropertyDiffResult> get_property_diff(const String &p_name) const;
	ObjectDiffResult();
	ObjectDiffResult(Object *p_old_object, Object *p_new_object, const Dictionary &p_property_diffs);
};

class NodeDiffResult : public RefCounted {
	GDCLASS(NodeDiffResult, RefCounted)

private:
	NodePath path;
	String type;
	Object *old_object;
	Object *new_object;
	Ref<ObjectDiffResult> props;

protected:
	static void _bind_methods();

public:
	void set_path(const NodePath &p_path);
	NodePath get_path() const;
	void set_type(const String &p_type);
	String get_type() const;
	void set_props(const Ref<ObjectDiffResult> &p_props);
	void set_old_object(Object *p_old_object);
	Object *get_old_object() const;
	void set_new_object(Object *p_new_object);
	Object *get_new_object() const;
	Ref<ObjectDiffResult> get_props() const;
	NodeDiffResult();
	NodeDiffResult(const NodePath &p_path, const String &p_type, Object *p_old_object, Object *p_new_object, const Ref<ObjectDiffResult> &p_props);
};

class PropertyDiffResult : public RefCounted {
	GDCLASS(PropertyDiffResult, RefCounted)

private:
	Object *old_object;
	Object *new_object;
	String name;
	String change_type;

	Variant old_value;
	Variant new_value;

protected:
	static void _bind_methods();

public:
	PropertyDiffResult();
	void set_name(const String &p_name);
	String get_name() const;
	void set_change_type(const String &p_change_type);
	String get_change_type() const;
	void set_old_value(const Variant &p_old_value);
	Variant get_old_value() const;
	void set_new_value(const Variant &p_new_value);
	Variant get_new_value() const;
	void set_old_object(Object *p_old_object);
	Object *get_old_object() const;
	void set_new_object(Object *p_new_object);
	Object *get_new_object() const;
	PropertyDiffResult(const String &p_name, const String &p_change_type, const Variant &p_old_value, const Variant &p_new_value, Object *p_old_object, Object *p_new_object);
};

#endif // DIFF_RESULT_H

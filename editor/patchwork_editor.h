#pragma once
#include "core/object/class_db.h"
#include "editor/editor_node.h"
#include "missing_resource_container.h"
#include "scene/main/node.h"

class PatchworkEditor : public Node {
	GDCLASS(PatchworkEditor, Node);

private:
	Ref<FakeInspectorResource> inspector_resource;
	EditorNode *editor = nullptr;
	static PatchworkEditor *singleton;
	PatchworkEditor *get_singleton();
	void _on_filesystem_changed();
	void _on_resources_reloaded();
	void _on_history_changed();
	void handle_change(const String &resource_path, const NodePath &node_path, HashMap<String, Variant> properties);
	void _on_file_changed(Dictionary dict);
	static bool unsaved_files_open();
	static bool detect_utf8(const PackedByteArray &p_utf8_buf);
	static Vector<String> get_recursive_dir_list(const String &p_dir, const Vector<String> &wildcards = {}, bool absolute = true, const String &rel = "");
	static void progress_add_task(const String &p_task, const String &p_label, int p_steps, bool p_can_cancel = false);
	static bool progress_task_step(const String &p_task, const String &p_state, int p_step = -1, bool p_force_refresh = true);
	static void progress_end_task(const String &p_task);

	static void progress_add_task_bg(const String &p_task, const String &p_label, int p_steps);
	static void progress_task_step_bg(const String &p_task, int p_step = -1);
	static void progress_end_task_bg(const String &p_task);
	static Dictionary get_diff(Dictionary changed_files_dict);
	static Dictionary get_file_diff(const String &p_path, const String &p_path2);
	static bool deep_equals(Variant a, Variant b, bool exclude_non_storage = true);
	static Dictionary get_diff_obj(Object *a, Object *b, bool exclude_non_storage = true);
	static Dictionary evaluate_node_differences(Node *scene1, Node *scene2, const NodePath &path);
	static Dictionary get_diff_res(Ref<Resource> p_res, Ref<Resource> p_res2);

protected:
	void _notification(int p_what);
	static void _bind_methods();

public:
	PatchworkEditor(EditorNode *p_editor);

public:
	PatchworkEditor();
	~PatchworkEditor();
};

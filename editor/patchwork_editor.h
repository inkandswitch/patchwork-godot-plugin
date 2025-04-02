#ifndef PATCHWORK_EDITOR_H
#define PATCHWORK_EDITOR_H

#include "core/io/resource_importer.h"
#include "core/object/ref_counted.h"
#include "core/variant/dictionary.h"
#include "core/variant/variant.h"
#include "editor/editor_node.h"
#include "scene/gui/control.h"
#include "scene/main/node.h"

class DiffResult;
class FileDiffResult;
class ObjectDiffResult;
class NodeDiffResult;

class PatchworkEditor : public Node {
	GDCLASS(PatchworkEditor, Node);

private:
	EditorNode *editor = nullptr;
	static PatchworkEditor *singleton;
	static void _editor_init_callback_static();

protected:
	void _notification(int p_what);
	static void _bind_methods();

public:
	static PatchworkEditor *get_singleton();
	PatchworkEditor(EditorNode *p_editor);
	PatchworkEditor();
	~PatchworkEditor();

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
	static Ref<DiffResult> get_diff(Dictionary changed_files_dict);
	static Ref<FileDiffResult> get_file_diff(const String &p_path, const String &p_path2, const Dictionary &p_options);
	static Ref<ObjectDiffResult> get_diff_obj(Object *a, Object *b, bool exclude_non_storage = true, const Dictionary &p_structured_changes = Dictionary());
	static Ref<NodeDiffResult> evaluate_node_differences(Node *scene1, Node *scene2, const NodePath &path, const Dictionary &p_options);
	static Ref<FileDiffResult> get_diff_res(Ref<Resource> p_res, Ref<Resource> p_res2, const Dictionary &p_options);
	static Ref<ResourceImporter> get_importer_by_name(const String &p_name);
	static Ref<Resource> import_and_load_resource(const String &p_path);
	static bool deep_equals(Variant a, Variant b, bool exclude_non_storage = true);

	static bool is_editor_importing();
	static void save_all_scenes_and_scripts();
	static void save_all_scripts();
	static PackedStringArray get_unsaved_scripts();
	static void reload_scripts(bool b_refresh_only = false);
};

#endif // PATCHWORK_EDITOR_H

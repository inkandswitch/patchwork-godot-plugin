#ifndef PATCHWORK_EDITOR_H
#define PATCHWORK_EDITOR_H

#include "core/object/ref_counted.h"
#include "core/io/resource_importer.h"
#include "core/variant/dictionary.h"
#include "core/variant/variant.h"
#include "editor/editor_node.h"
#include "scene/gui/control.h"
#include "scene/main/node.h"

class PatchworkEditor : public Node {
	GDCLASS(PatchworkEditor, Node);

private:
	EditorNode *editor = nullptr;
	static PatchworkEditor *singleton;
	static void _editor_init_callback_static();
	static Callable steal_close_current_script_tab_file_callback();

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
	static Ref<ResourceImporter> get_importer_by_name(const String &p_name);
	// TODO: remove this once the resource loader is working
	static String import_and_save_resource_to_temp(const String &p_path);
	static Error import_and_save_resource(const String &p_path, const String &import_file_content, const String &import_base_path);

	static Ref<Resource> import_and_load_resource(const String &p_path, const String &import_file_content, const String &import_base_path);
	static Vector<String> get_unsaved_files();

	static bool is_editor_importing();
	static bool is_changing_scene();
	static void save_all_scenes_and_scripts();
	static void save_all_scripts();
	static PackedStringArray get_unsaved_scripts();
	static void reload_scripts(PackedStringArray p_scripts);
	static void force_refresh_editor_inspector();
	static void open_script_file(const String &p_script);
	static String get_resource_script_class(const String &p_path);
	static void close_scene_file(const String &p_path);
	static void close_script_file(const String &p_path);
	static void close_files_if_open(const Vector<String> &p_paths);

	static void clear_editor_selection();

	static void refresh_after_source_change();
};

#endif // PATCHWORK_EDITOR_H

#include "patchwork_editor.h"
#include "core/variant/callable.h"
#include "core/variant/callable_bind.h"
#include "core/version_generated.gen.h"
#include "editor/debugger/editor_debugger_node.h"
#if GODOT_VERSION_MAJOR == 4 && GODOT_VERSION_MINOR < 5
#include "editor/plugins/shader_editor_plugin.h"
#include <editor/editor_file_system.h>
#include <editor/editor_inspector.h>
#include <editor/plugins/script_editor_plugin.h>
#else
#include "editor/shader/shader_editor_plugin.h"
#include <editor/file_system/editor_file_system.h>
#include <editor/inspector/editor_inspector.h>
#include <editor/script/script_editor_plugin.h>
#endif
#include "scene/gui/box_container.h"

#include <core/io/json.h>
#include <core/io/missing_resource.h>
#include <core/variant/variant.h>
#include <editor/editor_interface.h>
#include <editor/editor_undo_redo_manager.h>
#include <main/main.h>
#include <modules/gdscript/gdscript.h>
#include <scene/resources/packed_scene.h>

PatchworkEditor::PatchworkEditor() {
}

PatchworkEditor::~PatchworkEditor() {
}

PatchworkEditor *PatchworkEditor::get_singleton() {
	return singleton;
}

void PatchworkEditor::_on_filesystem_changed() {
}

void PatchworkEditor::_on_resources_reloaded() {
}

void PatchworkEditor::_on_history_changed() {
	// // get the current edited scene
	// auto scene = EditorNode::get_singleton()->get_edited_scene();
	// if (scene == nullptr) {
	// 	return;
	// }
	// // pack the scene into a packed scene
	// auto packed_scene = memnew(PackedScene);
	// packed_scene->pack(scene);
	// // temp file name with random name
	// auto temp_file = "user://temp_" + itos(OS::get_singleton()->get_unix_time()) + ".tscn";
	// Error err = ResourceSaver::save(packed_scene, temp_file);
	// if (err != OK) {
	// 	print_line("Error saving scene");
	// 	return;
	// }
	// // open the file
	// auto file = FileAccess::open(temp_file, FileAccess::READ);
	// if (file.is_valid()) {
	// 	auto contents = file->get_as_text();
	// 	auto scene_path = scene->get_scene_file_path();
	// 	if (scene_path == "res://main.tscn") {
	// 		fs->save_file(scene->get_scene_file_path(), contents);
	// 		// test getting the file
	// 		// auto file_contents = fs->get_file(scene->get_scene_file_path());
	// 		// if (file_contents != contents) {
	// 		// 	print_line("File contents do not match");
	// 		// } else {
	// 		// 	print_line("Yay");
	// 		// }
	// 	}
	// 	file->close();
	// }
	// DirAccess::remove_absolute(temp_file);
}

void PatchworkEditor::handle_change(const String &resource_path, const NodePath &node_path, HashMap<String, Variant> properties) {
	// auto res = ResourceLoader::load(resource_path);
	// if (!node_path.is_empty()) {
	// 	Ref<PackedScene> scene = res;
	// 	auto node_idx = scene->get_state()->find_node_by_path(node_path);
	// }
}

void PatchworkEditor::_on_file_changed(Dictionary dict) {
	// let args = ["file_path", "res://main.tscn",
	// "node_path", node_path.as_str(),
	// "type", "node_deleted",
	// ];
	// auto file_path = dict["file_path"];
	// auto node_path = dict["node_path"];
}

void PatchworkEditor::_notification(int p_what) {
	switch (p_what) {
		case NOTIFICATION_READY: {
			print_line("Entered tree");
			break;
		}
		default:
			break;
	}
}

bool PatchworkEditor::unsaved_files_open() {
	if (get_unsaved_scripts().size() > 0) {
		return true;
	}
	auto opened_scenes = EditorNode::get_editor_data().get_edited_scenes();
	for (int i = 0; i < opened_scenes.size(); i++) {
		auto id = opened_scenes[i].history_id;
		if (EditorUndoRedoManager::get_singleton()->is_history_unsaved(id)) {
			return true;
		}
	}
	// Not bound
	if (EditorUndoRedoManager::get_singleton()->is_history_unsaved(EditorUndoRedoManager::GLOBAL_HISTORY)) {
		return true;
	}

	return false;
}

Vector<String> PatchworkEditor::get_unsaved_files() {
	auto files = get_unsaved_scripts();
	auto opened_scenes = EditorNode::get_editor_data().get_edited_scenes();
	for (auto &scene : opened_scenes) {
		if (EditorUndoRedoManager::get_singleton()->is_history_unsaved(scene.history_id)) {
			files.append(scene.path);
		}
	}
	return files;
}

bool PatchworkEditor::detect_utf8(const PackedByteArray &p_utf8_buf) {
	int cstr_size = 0;
	int str_size = 0;
	const char *p_utf8 = (const char *)p_utf8_buf.ptr();
	int p_len = p_utf8_buf.size();
	if (p_len == 0) {
		return true; // empty string
	}
	bool p_skip_cr = false;
	/* HANDLE BOM (Byte Order Mark) */
	if (p_len < 0 || p_len >= 3) {
		bool has_bom = uint8_t(p_utf8[0]) == 0xef && uint8_t(p_utf8[1]) == 0xbb && uint8_t(p_utf8[2]) == 0xbf;
		if (has_bom) {
			//8-bit encoding, byte order has no meaning in UTF-8, just skip it
			if (p_len >= 0) {
				p_len -= 3;
			}
			p_utf8 += 3;
		}
	}

	// bool decode_error = false;
	// bool decode_failed = false;
	{
		const char *ptrtmp = p_utf8;
		const char *ptrtmp_limit = p_len >= 0 ? &p_utf8[p_len] : nullptr;
		int skip = 0;
		uint8_t c_start = 0;
		while (ptrtmp != ptrtmp_limit && *ptrtmp) {
#if CHAR_MIN == 0
			uint8_t c = *ptrtmp;
#else
			uint8_t c = *ptrtmp >= 0 ? *ptrtmp : uint8_t(256 + *ptrtmp);
#endif

			if (skip == 0) {
				if (p_skip_cr && c == '\r') {
					ptrtmp++;
					continue;
				}
				/* Determine the number of characters in sequence */
				if ((c & 0x80) == 0) {
					skip = 0;
				} else if ((c & 0xe0) == 0xc0) {
					skip = 1;
				} else if ((c & 0xf0) == 0xe0) {
					skip = 2;
				} else if ((c & 0xf8) == 0xf0) {
					skip = 3;
				} else if ((c & 0xfc) == 0xf8) {
					skip = 4;
				} else if ((c & 0xfe) == 0xfc) {
					skip = 5;
				} else {
					skip = 0;
					// print_unicode_error(vformat("Invalid UTF-8 leading byte (%x)", c), true);
					// decode_failed = true;
					return false;
				}
				c_start = c;

				if (skip == 1 && (c & 0x1e) == 0) {
					// print_unicode_error(vformat("Overlong encoding (%x ...)", c));
					// decode_error = true;
					return false;
				}
				str_size++;
			} else {
				if ((c_start == 0xe0 && skip == 2 && c < 0xa0) || (c_start == 0xf0 && skip == 3 && c < 0x90) || (c_start == 0xf8 && skip == 4 && c < 0x88) || (c_start == 0xfc && skip == 5 && c < 0x84)) {
					// print_unicode_error(vformat("Overlong encoding (%x %x ...)", c_start, c));
					// decode_error = true;
					return false;
				}
				if (c < 0x80 || c > 0xbf) {
					// print_unicode_error(vformat("Invalid UTF-8 continuation byte (%x ... %x ...)", c_start, c), true);
					// decode_failed = true;
					return false;

					// skip = 0;
				} else {
					--skip;
				}
			}

			cstr_size++;
			ptrtmp++;
		}
		// not checking for last sequence because we pass in incomplete bytes
		// if (skip) {
		// print_unicode_error(vformat("Missing %d UTF-8 continuation byte(s)", skip), true);
		// decode_failed = true;
		// return false;
		// }
	}

	if (str_size == 0) {
		// clear();
		return true; // empty string
	}

	// resize(str_size + 1);
	// char32_t *dst = ptrw();
	// dst[str_size] = 0;

	int skip = 0;
	uint32_t unichar = 0;
	while (cstr_size) {
#if CHAR_MIN == 0
		uint8_t c = *p_utf8;
#else
		uint8_t c = *p_utf8 >= 0 ? *p_utf8 : uint8_t(256 + *p_utf8);
#endif

		if (skip == 0) {
			if (p_skip_cr && c == '\r') {
				p_utf8++;
				continue;
			}
			/* Determine the number of characters in sequence */
			if ((c & 0x80) == 0) {
				// *(dst++) = c;
				unichar = 0;
				skip = 0;
			} else if ((c & 0xe0) == 0xc0) {
				unichar = (0xff >> 3) & c;
				skip = 1;
			} else if ((c & 0xf0) == 0xe0) {
				unichar = (0xff >> 4) & c;
				skip = 2;
			} else if ((c & 0xf8) == 0xf0) {
				unichar = (0xff >> 5) & c;
				skip = 3;
			} else if ((c & 0xfc) == 0xf8) {
				unichar = (0xff >> 6) & c;
				skip = 4;
			} else if ((c & 0xfe) == 0xfc) {
				unichar = (0xff >> 7) & c;
				skip = 5;
			} else {
				// *(dst++) = _replacement_char;
				// unichar = 0;
				// skip = 0;
				return false;
			}
		} else {
			if (c < 0x80 || c > 0xbf) {
				// *(dst++) = _replacement_char;
				skip = 0;
			} else {
				unichar = (unichar << 6) | (c & 0x3f);
				--skip;
				if (skip == 0) {
					if (unichar == 0) {
						return false;
						// print_unicode_error("NUL character", true);
						// decode_failed = true;
						// unichar = _replacement_char;
					} else if ((unichar & 0xfffff800) == 0xd800) {
						return false;

						// print_unicode_error(vformat("Unpaired surrogate (%x)", unichar), true);
						// decode_failed = true;
						// unichar = _replacement_char;
					} else if (unichar > 0x10ffff) {
						return false;

						// print_unicode_error(vformat("Invalid unicode codepoint (%x)", unichar), true);
						// decode_failed = true;
						// unichar = _replacement_char;
					}
					// *(dst++) = unichar;
				}
			}
		}

		cstr_size--;
		p_utf8++;
	}
	if (skip) {
		// return false;
		// *(dst++) = 0x20;
	}

	return true;
}

Vector<String> PatchworkEditor::get_recursive_dir_list(const String &p_dir, const Vector<String> &wildcards, const bool absolute, const String &rel) {
	Vector<String> ret;
	Error err;
	Ref<DirAccess> da = DirAccess::open(p_dir.path_join(rel), &err);
	ERR_FAIL_COND_V_MSG(da.is_null(), ret, "Failed to open directory " + p_dir);

	if (da.is_null()) {
		return ret;
	}
	Vector<String> dirs;
	Vector<String> files;

	String base = absolute ? p_dir : "";
	da->list_dir_begin();
	String f = da->get_next();
	while (!f.is_empty()) {
		if (f == "." || f == "..") {
			f = da->get_next();
			continue;
		} else if (da->current_is_dir()) {
			dirs.push_back(f);
		} else {
			files.push_back(f);
		}
		f = da->get_next();
	}
	da->list_dir_end();

	dirs.sort_custom<FileNoCaseComparator>();
	files.sort_custom<FileNoCaseComparator>();
	for (auto &d : dirs) {
		ret.append_array(get_recursive_dir_list(p_dir, wildcards, absolute, rel.path_join(d)));
	}
	for (auto &file : files) {
		if (wildcards.size() > 0) {
			for (int i = 0; i < wildcards.size(); i++) {
				if (file.get_file().matchn(wildcards[i])) {
					ret.append(base.path_join(rel).path_join(file));
					break;
				}
			}
		} else {
			ret.append(base.path_join(rel).path_join(file));
		}
	}

	return ret;
}

void PatchworkEditor::progress_add_task(const String &p_task, const String &p_label, int p_steps, bool p_can_cancel) {
	EditorNode::get_singleton()->progress_add_task(p_task, p_label, p_steps, p_can_cancel);
}

bool PatchworkEditor::progress_task_step(const String &p_task, const String &p_state, int p_step, bool p_force_refresh) {
	return EditorNode::get_singleton()->progress_task_step(p_task, p_state, p_step, p_force_refresh);
}

void PatchworkEditor::progress_end_task(const String &p_task) {
	EditorNode::get_singleton()->progress_end_task(p_task);
}
void PatchworkEditor::progress_add_task_bg(const String &p_task, const String &p_label, int p_steps) {
	EditorNode::get_singleton()->progress_add_task_bg(p_task, p_label, p_steps);
}
void PatchworkEditor::progress_task_step_bg(const String &p_task, int p_step) {
	EditorNode::get_singleton()->progress_task_step_bg(p_task, p_step);
}
void PatchworkEditor::progress_end_task_bg(const String &p_task) {
	EditorNode::get_singleton()->progress_end_task_bg(p_task);
}
String PatchworkEditor::get_resource_script_class(const String &p_path) {
	return ResourceLoader::get_resource_script_class(p_path);
}

Ref<ResourceImporter> PatchworkEditor::get_importer_by_name(const String &p_name) {
	return ResourceFormatImporter::get_singleton()->get_importer_by_name(p_name);
}

inline Vector<String> _get_section_keys(const Ref<ConfigFile> &p_config_file, const String &p_section) {
#if GODOT_VERSION_MAJOR == 4 && GODOT_VERSION_MINOR < 5
	List<String> param_keys;
	p_config_file->get_section_keys(p_section, &param_keys);
	Vector<String> param_keys_vector;
	for (auto &param_key : param_keys) {
		param_keys_vector.push_back(param_key);
	}
	return param_keys_vector;
#else
	return p_config_file->get_section_keys(p_section);
#endif
}

String PatchworkEditor::import_and_save_resource_to_temp(const String &p_path) {
	// get the import path
	auto import_path = p_path + ".import";
	// load the import file
	Ref<ConfigFile> import_file;
	import_file.instantiate();
	Error err = import_file->load(import_path);
	ERR_FAIL_COND_V_MSG(err != OK, {}, "Failed to load import file at path " + import_path);
	// get the importer name
	;
	String import_base_path = import_file->get_value("remap", "path", "");
	if (import_base_path.is_empty()) {
		// iterate through the remap keys, find one that begins with 'path'
		Vector<String> remap_keys = _get_section_keys(import_file, "remap");
		for (auto &remap_key : remap_keys) {
			if (remap_key.begins_with("path")) {
				import_base_path = import_file->get_value("remap", remap_key);
				break;
			}
		}
	}
	err = import_and_save_resource(p_path, FileAccess::get_file_as_string(import_path), import_base_path);
	ERR_FAIL_COND_V_MSG(err != OK, {}, "Failed to import resource at path " + p_path);
	return import_base_path;
}


Error PatchworkEditor::import_and_save_resource(const String &p_path, const String &import_file_content, const String &import_base_path) {
	String base_dir = import_base_path.get_base_dir();
	HashMap<StringName, Variant> params;

	Ref<ConfigFile> import_file;
	import_file.instantiate();

	Error err = import_file->parse(import_file_content);
	ERR_FAIL_COND_V_MSG(err != OK, err, "Failed to parse import file content");
	String importer_name = import_file->get_value("remap", "importer");
	Vector<String> param_keys = _get_section_keys(import_file, "params");
	for (auto &param_key : param_keys) {
		auto param_value = import_file->get_value("params", param_key);
		params[param_key] = param_value;
	}

	// make dir recursive
	DirAccess::make_dir_recursive_absolute(base_dir);
	auto importer = get_importer_by_name(importer_name);
	List<String> import_variants;
	List<String> import_options;
	Variant metadata;
	// set the default values for the import options in case they are not present in the import file
	List<ResourceImporter::ImportOption> opts;
	importer->get_import_options(p_path, &opts);
	for (const ResourceImporter::ImportOption &E : opts) {
		if (!params.has(E.option.name)) { //this one is not present
			params[E.option.name] = E.default_value;
		}
	}

	return importer->import(ResourceUID::INVALID_ID, p_path, import_base_path, params, &import_variants, &import_options, &metadata);
}

Ref<Resource> PatchworkEditor::import_and_load_resource(const String &p_path, const String &import_file_content, const String &import_base_path) {
	Error err = import_and_save_resource(p_path, import_file_content, import_base_path);
	ERR_FAIL_COND_V_MSG(err != OK, nullptr, "Failed to import resource at path " + p_path);
	return ResourceLoader::load(import_base_path, "", ResourceFormatLoader::CACHE_MODE_IGNORE_DEEP);
}

void PatchworkEditor::save_all_scenes_and_scripts() {
	ShaderEditorPlugin *shader_editor = Object::cast_to<ShaderEditorPlugin>(EditorNode::get_editor_data().get_editor_by_name("Shader"));
	if (shader_editor) {
		shader_editor->save_external_data();
	}
	save_all_scripts();
	// save the scenes
	EditorInterface::get_singleton()->save_all_scenes();
}

void PatchworkEditor::save_all_scripts() {
	EditorInterface::get_singleton()->get_script_editor()->save_all_scripts();
}

PackedStringArray PatchworkEditor::get_unsaved_scripts() {
	return EditorInterface::get_singleton()->get_script_editor()->get_unsaved_scripts();
}

void PatchworkEditor::reload_scripts(PackedStringArray p_scripts) {
	// Call deferred to make sure it runs on the main thread.
	print_line("Reloading scripts: " + String(", ").join(p_scripts));
	Array scripts;
	for (auto &script : p_scripts) {
		auto sc = ResourceLoader::load(script, "", ResourceFormatLoader::CACHE_MODE_REPLACE_DEEP);
		if (sc.is_valid()) {
			scripts.append(sc);
		}
	}
	// soft_reload = false means it will reload all the script instances too
	GDScriptLanguage::get_singleton()->reload_scripts(scripts, true);
	// now get all the open scripts in the editor
	auto script_editor = EditorInterface::get_singleton()->get_script_editor();
	for (auto &script : script_editor->get_open_scripts()) {
		// If it has one of these scripts open, we have to reload it
		if (p_scripts.has(script->get_path())) {
			// call it with refresh_only = false (Forces it to reload the text)
			script_editor->reload_scripts(false);
			break;
		}
	}
	EditorDebuggerNode::get_singleton()->reload_scripts(p_scripts);
	// EditorInterface::get_singleton()->get_script_editor()->reload_scripts(p_scripts);
}

void PatchworkEditor::open_script_file(const String &p_script) {
	EditorInterface::get_singleton()->get_script_editor()->open_file(p_script);
}

void PatchworkEditor::force_refresh_editor_inspector() {
	EditorInterface::get_singleton()->get_inspector()->update_tree();
}

// not bound
bool PatchworkEditor::is_editor_importing() {
	return EditorFileSystem::get_singleton()->is_importing();
}

bool PatchworkEditor::is_changing_scene() {
	return EditorNode::get_singleton()->is_changing_scene();
}

void PatchworkEditor::clear_editor_selection() {
	EditorNode::get_singleton()->get_editor_selection()->clear();
}

void PatchworkEditor::refresh_after_source_change() {
	EditorFileSystem::get_singleton()->scan_changes();
	// TODO: make this take in scripts to reload
	ScriptEditor::get_singleton()->reload_scripts();

	Main::iteration();

	while (EditorFileSystem::get_singleton()->is_scanning()) {
		OS::get_singleton()->delay_usec(10000);
		Main::iteration();
	}

	auto current_scene = EditorInterface::get_singleton()->get_edited_scene_root();

	auto open_scenes = EditorInterface::get_singleton()->get_open_scenes();
	for (auto &scene : open_scenes) {
		if (current_scene != nullptr && scene == current_scene->get_scene_file_path()) {
			continue;
		}
		while (is_changing_scene()) {
			OS::get_singleton()->delay_usec(10000);
			Main::iteration();
		}
		EditorInterface::get_singleton()->reload_scene_from_path(scene);
	}
	if (current_scene != nullptr) {
		// always iterate once if we must switch back, because sometimes is_changing_scene is false but we still need to iterate (?!)
		do {
			OS::get_singleton()->delay_usec(10000);
			Main::iteration();
		} while (is_changing_scene());
		EditorInterface::get_singleton()->reload_scene_from_path(current_scene->get_scene_file_path());
	}
}

Callable PatchworkEditor::steal_close_current_script_tab_file_callback() {
	ScriptEditor *script_editor = EditorInterface::get_singleton()->get_script_editor();

	ERR_FAIL_COND_V_MSG(script_editor == nullptr, Callable(), "No script editor found");
	Callable new_callable;
	for (auto &child : script_editor->get_children()) {
		if (Object::cast_to<ConfirmationDialog>(child)) {
			auto confirmation_dialog = Object::cast_to<ConfirmationDialog>(child);
			bool found = false;
			for (auto &child : confirmation_dialog->get_children()) {
				if (Object::cast_to<HBoxContainer>(child)) {
					for (auto &subchild : Object::cast_to<HBoxContainer>(child)->get_children()) {
						if (Object::cast_to<Button>(subchild)) {
							auto button = Object::cast_to<Button>(subchild);
							if (button->get_text() == TTR("Discard")) {
								found = true;
								break;
							}
						}
					}
				}
				if (found) {
					break;
				}
			}
			ERR_FAIL_COND_V_MSG(!found, Callable(), "No discard button found");

			// we have to steal the signal handler for the "confirmed" signal
			List<Connection> connections;
			confirmation_dialog->get_signal_connection_list(SceneStringName(confirmed), &connections);
			ERR_FAIL_COND_V_MSG(connections.is_empty(), Callable(), "No connection found for the confirmed button");
			Callable confirm_callback = connections.front()->get().callable;
			// The signal handler is bound with arguments that we don't want, so we need to unbind it
			// we need "false, false" so that closing it does not save it and doesn't mess with the history
			auto custom = confirm_callback.get_custom();
			ERR_FAIL_COND_V_MSG(custom == nullptr, Callable(), "No callable found for the confirmed button");
			CallableCustomBind *custom_bind = dynamic_cast<CallableCustomBind *>(custom);
			ERR_FAIL_COND_V_MSG(custom_bind == nullptr, Callable(), "No callable found for the confirmed button");
			new_callable = custom_bind->get_callable().bind(false, false);
			ERR_FAIL_COND_V_MSG(new_callable.is_null(), Callable(), "Could not rebind the confirmed button");
			break;
		}
	}
	return new_callable;
}

void PatchworkEditor::close_script_file(const String &p_path) {
	auto scripts = EditorInterface::get_singleton()->get_script_editor()->get_open_scripts();
	Ref<Script> found;
	for (auto &script : scripts) {
		if (script->get_path() == p_path) {
			found = script;
			break;
		}
	}
	if (!found.is_valid()) {
		return;
	}
	Callable close_current_script_tab_callback = steal_close_current_script_tab_file_callback();
	ERR_FAIL_COND_MSG(close_current_script_tab_callback.is_null(), "No close callback found");
	// first, we have to load it
	EditorInterface::get_singleton()->get_script_editor()->edit(found, 0, 0, false);
	close_current_script_tab_callback.call();

	// we have to steal _close_tab from the signal handler on the erase_tab_confirm in the script editor
}

void PatchworkEditor::close_scene_file(const String &p_path) {
	auto open_scenes = EditorInterface::get_singleton()->get_open_scenes();
	if (!open_scenes.has(p_path)) {
		return; // nothing to do
	}
	// We have to LOAD the scene first (if it's already open, it'll just switch to the tab) and then close it by doing EditorNode::get_singleton()->trigger_menu_option()

	EditorNode::get_singleton()->load_scene(p_path);

	// Main::iteration();
#if GODOT_VERSION_MAJOR == 4 && GODOT_VERSION_MINOR < 5
	constexpr int CLOSE_MENU_OPTION = EditorNode::FILE_CLOSE;
#else
	constexpr int CLOSE_MENU_OPTION = EditorNode::SCENE_CLOSE;
#endif

	// this needs to be bound to GDScript
	EditorNode::get_singleton()->trigger_menu_option(CLOSE_MENU_OPTION, true);
}

void PatchworkEditor::close_files_if_open(const Vector<String> &p_paths) {
	for (auto &path : p_paths) {
		auto ext = path.get_extension().to_lower();
		if (ext == "tscn" || ext == "scn") {
			close_scene_file(path);
		} else if (ext == "gd") {
			close_script_file(path);
		}
	}
}

PatchworkEditor *PatchworkEditor::singleton = nullptr;

PatchworkEditor::PatchworkEditor(EditorNode *p_editor) {
	singleton = this;
	editor = p_editor;
	// EditorUndoRedoManager::get_singleton()->connect(SNAME("history_changed"), callable_mp(this, &PatchworkEditor::_on_history_changed));
	//
	// fs = GodotProject::create("");
	// this->add_child(fs);
	// EditorFileSystem::get_singleton()->connect("filesystem_changed", callable_mp(this, &PatchworkEditor::signal_callback));
}

void PatchworkEditor::_bind_methods() {
	ClassDB::bind_static_method(get_class_static(), D_METHOD("progress_add_task", "task", "label", "steps", "can_cancel"), &PatchworkEditor::progress_add_task);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("progress_task_step", "task", "state", "step", "force_refresh"), &PatchworkEditor::progress_task_step);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("progress_end_task", "task"), &PatchworkEditor::progress_end_task);

	ClassDB::bind_static_method(get_class_static(), D_METHOD("progress_add_task_bg", "task", "label", "steps"), &PatchworkEditor::progress_add_task_bg);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("progress_task_step_bg", "task", "step"), &PatchworkEditor::progress_task_step_bg);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("progress_end_task_bg", "task"), &PatchworkEditor::progress_end_task_bg);

	ClassDB::bind_static_method(get_class_static(), "unsaved_files_open", &PatchworkEditor::unsaved_files_open);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("detect_utf8", "utf8_buf"), &PatchworkEditor::detect_utf8);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("get_recursive_dir_list", "dir", "wildcards", "absolute", "rel"), &PatchworkEditor::get_recursive_dir_list);

	ClassDB::bind_static_method(get_class_static(), D_METHOD("get_importer_by_name", "name"), &PatchworkEditor::get_importer_by_name);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("import_and_save_resource_to_temp", "path"), &PatchworkEditor::import_and_save_resource_to_temp);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("import_and_load_resource", "path", "import_file_content", "import_base_path"), &PatchworkEditor::import_and_load_resource);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("import_and_save_resource", "path", "import_file_content", "import_base_path"), &PatchworkEditor::import_and_save_resource);

	ClassDB::bind_static_method(get_class_static(), D_METHOD("save_all_scenes_and_scripts"), &PatchworkEditor::save_all_scenes_and_scripts);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("save_all_scripts"), &PatchworkEditor::save_all_scripts);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("get_unsaved_scripts"), &PatchworkEditor::get_unsaved_scripts);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("reload_scripts", "scripts"), &PatchworkEditor::reload_scripts);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("is_editor_importing"), &PatchworkEditor::is_editor_importing);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("is_changing_scene"), &PatchworkEditor::is_changing_scene);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("get_unsaved_files"), &PatchworkEditor::get_unsaved_files);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("force_refresh_editor_inspector"), &PatchworkEditor::force_refresh_editor_inspector);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("open_script_file", "script"), &PatchworkEditor::open_script_file);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("clear_editor_selection"), &PatchworkEditor::clear_editor_selection);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("get_resource_script_class", "path"), &PatchworkEditor::get_resource_script_class);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("close_scene_file", "path"), &PatchworkEditor::close_scene_file);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("close_script_file", "path"), &PatchworkEditor::close_script_file);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("close_files_if_open", "paths"), &PatchworkEditor::close_files_if_open);
	ClassDB::bind_static_method(get_class_static(), D_METHOD("refresh_after_source_change"), &PatchworkEditor::refresh_after_source_change);
}

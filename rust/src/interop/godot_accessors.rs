use godot::{
    builtin::{GString, PackedStringArray, Variant},
    classes::{ClassDb, EditorInterface, Object},
    meta::ToGodot,
    obj::Gd,
};
use godot::obj::Singleton;

use crate::interop::{godot_helpers::ToGodotExt, patchwork_config::PatchworkConfig};

/// Allows Rust code to easily get and set Patchwork configuration values via Godot's config system.
pub struct PatchworkConfigAccessor {}

impl PatchworkConfigAccessor {
    pub fn get_project_doc_id() -> String {
        PatchworkConfigAccessor::get_project_value("project_doc_id", "")
    }

    pub fn get_project_value(name: &str, default: &str) -> String {
        PatchworkConfig::singleton()
            .bind()
            .get_project_value(GString::from(name), default.to_variant())
            .to::<String>()
    }

    pub fn set_project_value(name: &str, value: &str) {
        PatchworkConfig::singleton()
            .bind_mut()
            .set_project_value(GString::from(name), value.to_variant());
    }

    pub fn get_user_value(name: &str, default: &str) -> String {
        PatchworkConfig::singleton()
            .bind()
            .get_user_value(GString::from(name), default.to_variant())
            .to::<String>()
    }

    #[allow(dead_code)] // will be used later
    pub fn set_user_value(name: &str, value: &str) {
        PatchworkConfig::singleton()
            .bind_mut()
            .set_user_value(GString::from(name), value.to_variant());
    }
}

/// Allows Rust code to access the C++ PatchworkEditor editor module from Godot.
pub struct PatchworkEditorAccessor {}

#[allow(dead_code)] // entire API might not be used yet
impl PatchworkEditorAccessor {
    pub fn import_and_save_resource(path: &str, import_file_content: &str, import_base_path: &str) -> godot::global::Error {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "import_and_save_resource",
            &[path.to_variant(), import_file_content.to_variant(), import_base_path.to_variant()],
        ).to::<godot::global::Error>()
    }

    pub fn is_editor_importing() -> bool {
        return ClassDb::singleton()
            .class_call_static("PatchworkEditor", "is_editor_importing", &[])
            .to::<bool>();
    }

    pub fn is_changing_scene() -> bool {
        return ClassDb::singleton()
            .class_call_static("PatchworkEditor", "is_changing_scene", &[])
            .to::<bool>();
    }

    pub fn reload_scripts(scripts: &Vec<String>) {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "reload_scripts",
            &[scripts.to_variant()],
        );
    }

    pub fn force_refresh_editor_inspector() {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "force_refresh_editor_inspector",
            &[],
        );
    }

    pub fn progress_add_task(task: &str, label: &str, steps: i32, can_cancel: bool) {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "progress_add_task",
            &[
                task.to_variant(),
                label.to_variant(),
                steps.to_variant(),
                can_cancel.to_variant(),
            ],
        );
    }

    pub fn progress_task_step(task: &str, state: &str, step: i32, force_refresh: bool) {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "progress_task_step",
            &[
                task.to_variant(),
                state.to_variant(),
                step.to_variant(),
                force_refresh.to_variant(),
            ],
        );
    }

    pub fn progress_end_task(task: &str) {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "progress_end_task",
            &[task.to_variant()],
        );
    }

    pub fn unsaved_files_open() -> bool {
        ClassDb::singleton()
            .class_call_static("PatchworkEditor", "unsaved_files_open", &[])
            .to::<bool>()
    }

    pub fn clear_editor_selection() {
        ClassDb::singleton().class_call_static("PatchworkEditor", "clear_editor_selection", &[]);
    }

    pub fn close_scene_file(path: &str) {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "close_scene_file",
            &[path.to_variant()],
        );
    }

    pub fn close_files_if_open(paths: &Vec<String>) {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "close_files_if_open",
            &[paths.to_variant()],
        );
    }

    pub fn refresh_after_source_change() {
        ClassDb::singleton().class_call_static(
            "PatchworkEditor",
            "refresh_after_source_change",
            &[],
        );
    }
}

/// Allows Rust code to access the Godot EditorFilesystem API
pub struct EditorFilesystemAccessor {}

#[allow(dead_code)] // entire API might not be used yet
impl EditorFilesystemAccessor {
    pub fn is_scanning() -> bool {
        EditorInterface::singleton()
            .get_resource_filesystem()
            .map(|fs| return fs.is_scanning())
            .unwrap_or(false)
    }

    pub fn reimport_files(files: &Vec<String>) {
        let files_packed = files
            .iter()
            .map(|f| GString::from(f))
            .collect::<PackedStringArray>();
        EditorInterface::singleton()
            .get_resource_filesystem()
            .unwrap()
            .reimport_files(&files_packed);
    }

    pub fn reload_scene_from_path(path: &str) {
        EditorInterface::singleton().reload_scene_from_path(&GString::from(path));
    }

    pub fn scan() {
        EditorInterface::singleton()
            .get_resource_filesystem()
            .unwrap()
            .scan();
    }

    pub fn scan_changes() {
        EditorInterface::singleton()
            .get_resource_filesystem()
            .unwrap()
            .scan_sources();
    }

    pub fn get_inspector_edited_object() -> Option<Gd<Object>> {
        EditorInterface::singleton()
            .get_inspector()
            .unwrap()
            .get_edited_object()
    }

    pub fn clear_inspector_item() {
        let object = Gd::<Object>::null_arg();
        EditorInterface::singleton()
            .inspect_object_ex(object)
            .for_property("")
            .inspector_only(true)
            .done();
    }
}

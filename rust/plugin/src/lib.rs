use godot::prelude::*;

struct MyExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {}

mod doc_handle_map;
mod doc_utils;
mod godot_project;
mod godot_scene;
mod godot_project_driver;
mod utils;

pub(crate) use doc_handle_map::DocHandleMap;

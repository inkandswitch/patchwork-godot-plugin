use godot::prelude::*;

struct MyExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {}

mod godot_project;
mod godot_scene;
mod doc_handle_map;
pub(crate) use doc_handle_map::DocHandleMap;

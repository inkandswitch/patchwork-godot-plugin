use godot::prelude::*;

struct MyExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {}

mod doc_handle_map;
mod doc_state_map;
mod godot_project;
mod godot_scene;

pub(crate) use doc_handle_map::DocHandleMap;

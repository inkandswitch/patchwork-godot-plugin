use godot::prelude::*;

struct MyExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {}

mod doc_utils;
mod godot_project;
mod godot_project_driver;
mod godot_scene;
mod utils;

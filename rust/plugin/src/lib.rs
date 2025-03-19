use godot::prelude::*;

struct MyExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {}

mod doc_utils;
mod godot_parser;
mod godot_project;
mod godot_project_driver;
mod patches;
mod utils;

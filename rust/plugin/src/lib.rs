use godot::prelude::*;

struct MyExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {}

mod doc_utils;
pub mod godot_parser;
pub mod godot_project;
mod godot_project_driver;
mod patches;
pub mod utils;

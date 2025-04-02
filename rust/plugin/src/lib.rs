use godot::{classes::{ClassDb, Engine}, meta::ParamType, prelude::*};
use godot_project::GodotProject;

struct MyExtension;
const SINGLETON_NAME: &str = "GodotProject";
#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {
	fn on_level_init(level: InitLevel) {
        if level == InitLevel::Scene {
            // The `&str` identifies your singleton and can be
            // used later to access it.
            Engine::singleton().register_singleton(
                SINGLETON_NAME,
                &GodotProject::new_alloc(),
            );
        } else if level == InitLevel::Editor {
			// get the singleton, add it as a child to the editor
			let callable: Callable = Callable::from_sync_fn("init_godot_project", |args| {
				let singleton = Engine::singleton().get_singleton(&StringName::from(SINGLETON_NAME)).unwrap().cast::<Node>();
				let mut editor_node = ClassDb::singleton().class_call_static("PatchworkEditor", "get_editor_node", &[]).to::<Gd<Node>>();
				editor_node.add_child(&singleton);
				Ok(Variant::nil())
			});
			ClassDb::singleton().class_call_static("PatchworkEditor", "add_editor_init_callback", &[callable.to_variant()]);
		}
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Scene {
            let mut engine = Engine::singleton();
            let singleton_name = SINGLETON_NAME;

            if let Some(my_singleton) = engine.get_singleton(singleton_name) {
                // Unregistering from Godot, and freeing from memory is required
                // to avoid memory leaks, warnings, and hot reloading problems.
                engine.unregister_singleton(singleton_name);
                my_singleton.free();
            } else {
                // You can either recover, or panic from here.
                godot_error!("Failed to get singleton");
            }
        }
    }
}

mod doc_utils;
pub mod godot_parser;
pub mod godot_project;
mod godot_project_driver;
mod patches;
pub mod utils;

use godot::{classes::{ClassDb, Engine}, init::EditorRunBehavior, meta::ParamType, prelude::*};
use godot_project::GodotProject;

struct MyExtension;
const SINGLETON_NAME: &str = "GodotProject";
#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {

	fn editor_run_behavior() -> EditorRunBehavior {
		EditorRunBehavior::ToolClassesOnly
	}

	fn on_level_init(level: InitLevel) {
        if level == InitLevel::Scene {
			println!("** on_level_init: Scene");
			if (Engine::singleton().has_singleton(&StringName::from(SINGLETON_NAME))) {
				Engine::singleton().unregister_singleton(&StringName::from(SINGLETON_NAME));
				if let Some(my_singleton) = Engine::singleton().get_singleton(&StringName::from(SINGLETON_NAME)) {
					my_singleton.free();
				}
			}
            Engine::singleton().register_singleton(
                SINGLETON_NAME,
                &GodotProject::new_alloc(),
            );
        } else if level == InitLevel::Editor {
			println!("** on_level_init: Editor");
		}
    }

    fn on_level_deinit(level: InitLevel) {
		if level == InitLevel::Editor {
			println!("** on_level_deinit: Editor");
		}
        if level == InitLevel::Scene {
			println!("** on_level_deinit: Scene");
            let mut engine = Engine::singleton();
            let singleton_name = SINGLETON_NAME;

            if let Some(my_singleton) = engine.get_singleton(singleton_name) {
                // Unregistering from Godot, and freeing  from memory is required ddsafds
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

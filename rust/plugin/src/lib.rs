mod patchwork_config;
use godot::{classes::{ClassDb, Engine}, init::EditorRunBehavior, meta::ParamType, prelude::*};
use godot_project::GodotProject;
use patchwork_config::PatchworkConfig;
struct MyExtension;


fn unregister_singleton(singleton_name: &str) {
	if (Engine::singleton().has_singleton(singleton_name)) {
		let my_singleton = Engine::singleton().get_singleton(singleton_name);
		Engine::singleton().unregister_singleton(singleton_name);
		if let Some(my_singleton) = my_singleton {
			my_singleton.free();
		}
	}
}

#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {
	fn editor_run_behavior() -> EditorRunBehavior {
		EditorRunBehavior::ToolClassesOnly
	}

	fn on_level_init(level: InitLevel) {
        if level == InitLevel::Scene {
			println!("** on_level_init: Scene");
			Engine::singleton().register_singleton(
				"PatchworkConfig",
				&PatchworkConfig::new_alloc(),
			);

            Engine::singleton().register_singleton(
                "GodotProject",
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
            unregister_singleton("GodotProject");
			unregister_singleton("PatchworkConfig");
        }
    }
}

mod doc_utils;
pub mod godot_parser;
pub mod godot_project;
mod godot_project_driver;
mod patches;
pub mod utils;
mod file_system_driver;
mod file_utils;

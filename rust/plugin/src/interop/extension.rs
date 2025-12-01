use godot::{classes::Engine, init::{EditorRunBehavior, ExtensionLibrary, InitLevel, gdextension}, obj::NewAlloc};

use crate::{helpers::tracing::initialize_tracing, interop::{godot_project::GodotProject, patchwork_config::PatchworkConfig}};


struct MyExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MyExtension {
    fn editor_run_behavior() -> EditorRunBehavior {
        EditorRunBehavior::ToolClassesOnly
    }

    fn on_level_init(level: InitLevel) {
        if level == InitLevel::Scene {
            initialize_tracing();
            tracing::info!("** on_level_init: Scene");
            Engine::singleton()
                .register_singleton("PatchworkConfig", &PatchworkConfig::new_alloc());

            Engine::singleton().register_singleton("GodotProject", &GodotProject::new_alloc());
        } else if level == InitLevel::Editor {
            tracing::info!("** on_level_init: Editor");
        }
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Editor {
            tracing::info!("** on_level_deinit: Editor");
        }
        if level == InitLevel::Scene {
            tracing::info!("** on_level_deinit: Scene");
            unregister_singleton("GodotProject");
            unregister_singleton("PatchworkConfig");
        }
    }
}


fn unregister_singleton(singleton_name: &str) {
    if Engine::singleton().has_singleton(singleton_name) {
        let my_singleton = Engine::singleton().get_singleton(singleton_name);
        Engine::singleton().unregister_singleton(singleton_name);
        if let Some(my_singleton) = my_singleton {
            my_singleton.free();
        }
    }
}

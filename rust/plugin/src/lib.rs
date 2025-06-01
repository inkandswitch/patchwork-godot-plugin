mod patchwork_config;
use godot::{
    classes::{Engine, ProjectSettings},
    init::EditorRunBehavior,
    prelude::*,
};
use godot_project::GodotProject;
use patchwork_config::PatchworkConfig;
use tracing_appender::{non_blocking::{NonBlocking, WorkerGuard}, rolling};
use tracing_subscriber::{
    fmt::{self, format::Writer, time::FormatTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};
struct MyExtension;

mod custom_logging_fmt;
mod doc_utils;
mod file_system_driver;
mod file_utils;
mod godot_helpers;
pub mod godot_parser;
pub mod godot_project;
mod godot_project_driver;
mod patches;
pub mod utils;

fn unregister_singleton(singleton_name: &str) {
    if (Engine::singleton().has_singleton(singleton_name)) {
        let my_singleton = Engine::singleton().get_singleton(singleton_name);
        Engine::singleton().unregister_singleton(singleton_name);
        if let Some(my_singleton) = my_singleton {
            my_singleton.free();
        }
    }
}
struct CompactTime;
impl FormatTime for CompactTime {
    fn format_time(&self, w: &mut Writer<'_>) -> Result<(), std::fmt::Error> {
        write!(
            w,
            "{}",
            custom_logging_fmt::TimeNoDate::from(std::time::SystemTime::now())
        )
    }
}

fn get_user_dir() -> String {
    let user_dir = ProjectSettings::singleton()
        .globalize_path("user://")
        .to_string();
    user_dir
}

static mut m_file_writer_mutex: Option<WorkerGuard> = None;
fn initialize_tracing() {

    let file_appender = tracing_appender::rolling::daily(get_user_dir(), "patchwork.log");
    let (non_blocking_file_writer, _guard) = tracing_appender::non_blocking(file_appender);
	// if the mutex gets dropped, the file writer will be closed, so we need to keep it alive
	unsafe{m_file_writer_mutex = Some(_guard);}
    println!("!!! Logging to {:?}/patchwork.log", get_user_dir());
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_timer(CompactTime)
        .compact()
        .with_writer(custom_logging_fmt::CustomStdoutWriter::custom_stdout)
        .with_filter(EnvFilter::new("info")
        .add_directive("patchwork_rust_core=debug".parse().unwrap())
        .add_directive("automerge_repo=info".parse().unwrap()));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_line_number(true)
		.with_ansi(false)
        .with_writer(non_blocking_file_writer.clone())
        .with_filter(EnvFilter::new("info")
        .add_directive("patchwork_rust_core=trace".parse().unwrap())
        .add_directive("automerge_repo=debug".parse().unwrap()));
    if let Err(e) = tracing_subscriber::registry()
        // stdout writer
        .with(stdout_layer)
        // we want a file writer too
        .with(file_layer)
        .try_init()
    {
        tracing::error!("Failed to initialize tracing subscriber: {:?}", e);
    } else {
        tracing::info!("Tracing subscriber initialized");
    }
}

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

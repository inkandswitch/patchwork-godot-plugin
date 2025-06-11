use godot::classes::{ConfigFile, Engine, FileAccess};
use godot::global::Error;
use godot::prelude::*;
use godot::builtin::{Variant};

#[derive(GodotClass)]
#[class(no_init, base=Object)]
pub struct PatchworkConfig {
    #[base]
    base: Base<Object>,

    project_config: Gd<ConfigFile>,
    user_config: Gd<ConfigFile>,
}

const CONFIG_PROJECT_FILE: &str = "res://patchwork.cfg";
const USER_CONFIG_FILE: &str = "user://patchwork.cfg";

#[godot_api]
impl PatchworkConfig {

	pub fn singleton() -> Gd<Self> {
		Engine::singleton().get_singleton("PatchworkConfig").unwrap().cast::<Self>()
	}

    #[func]
    pub fn get_project_value(&self, key: GString, default: Variant) -> Variant {
        self.project_config
            .get_value_ex("patchwork", &key).default(&default).done()
    }

    #[func]
    pub fn get_user_value(&self, key: GString, default: Variant) -> Variant {
        self.user_config
            .get_value_ex("patchwork", &key).default(&default).done()
    }

    #[func]
    pub fn set_project_value(&mut self, key: GString, value: Variant) {
        self.project_config.set_value("patchwork", &key, &value);
        if self.project_config.save(CONFIG_PROJECT_FILE) != Error::OK {
            godot_error!("Failed to save patchwork configuration");
        }
    }

    #[func]
    pub fn set_user_value(&mut self, key: GString, value: Variant) {
        self.user_config.set_value("patchwork", &key, &value);
        if self.user_config.save(USER_CONFIG_FILE) != Error::OK{
            godot_error!("Failed to save patchwork user configuration");
        }
    }
}

#[godot_api]
impl IObject for PatchworkConfig {
    fn init(_base: Base<Object>) -> Self {
		let mut project_config = ConfigFile::new_gd();
		let mut user_config = ConfigFile::new_gd();
		if FileAccess::file_exists(CONFIG_PROJECT_FILE) {
			project_config.load(CONFIG_PROJECT_FILE);
		}
		if FileAccess::file_exists(USER_CONFIG_FILE) {
			user_config.load(USER_CONFIG_FILE);
		}
		Self {
			base: _base,
			project_config,
			user_config,
		}
	}
}
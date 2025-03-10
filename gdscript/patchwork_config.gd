class_name PatchworkConfig

const CONFIG_PROJECT_FILE = "res://patchwork.cfg"
const USER_CONFIG_FILE = "user://patchwork.cfg"
var _project_config = ConfigFile.new()
var _user_config = ConfigFile.new()

func _init():
    if _project_config.load(CONFIG_PROJECT_FILE) != OK:
        push_error("Failed to load patchwork configuration")
    if _user_config.load(USER_CONFIG_FILE) != OK:
        push_error("Failed to load patchwork user configuration")

func get_project_value(key: String, default = null):
    return _project_config.get_value("patchwork", key, default)

func get_user_value(key: String, default = null):
    return _user_config.get_value("patchwork", key, default)

func set_project_value(key: String, value) -> void:
    _project_config.set_value("patchwork", key, value)
    if _project_config.save(CONFIG_PROJECT_FILE) != OK:
        push_error("Failed to save patchwork configuration")

func set_user_value(key: String, value) -> void:
    _user_config.set_value("patchwork", key, value)
    if _user_config.save(USER_CONFIG_FILE) != OK:
        push_error("Failed to save patchwork user configuration")

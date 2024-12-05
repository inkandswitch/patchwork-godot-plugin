class_name PatchworkConfig

const CONFIG_FILE = "res://addons/patchwork/patchwork.cfg"
var _config = ConfigFile.new()

func _init():
    var err = _config.load(CONFIG_FILE)
    if err != OK:
        save_config()

func get_value(key: String, default = null):
    return _config.get_value("patchwork", key, default)

func set_value(key: String, value) -> void:
    _config.set_value("patchwork", key, value)
    save_config()

func save_config() -> void:
    var err = _config.save(CONFIG_FILE)
    if err != OK:
        push_error("Failed to save patchwork configuration")
@tool
class_name FileChangeListener

var file_contents: Dictionary = {}
var editor_plugin: EditorPlugin
var _ignore_changes = false

signal file_changed(path: String, file_name: String)

func _init(editor_plugin: EditorPlugin):
  self.editor_plugin = editor_plugin

  # listen to file system
  var file_system = editor_plugin.get_editor_interface().get_resource_filesystem()
  # file_system.connect("filesystem_changed", _on_filesystem_changed)
  # file_system.connect("resources_reload", _on_resources_reloaded)

  # listen to changes of scene file
  editor_plugin.get_undo_redo().connect("version_changed", _on_changed)
  editor_plugin.get_undo_redo().connect("history_changed", _on_changed)


func ignore_changes(callback: Callable) -> void:
  _ignore_changes = true
  callback.call()
  _ignore_changes = false
  
func stop():
  var file_system = editor_plugin.get_editor_interface().get_resource_filesystem()

  # Cleanup connections when plugin is disabled
  if file_system:
    pass
    #file_system.disconnect("filesystem_changed", _on_filesystem_changed)
    #file_system.disconnect("resources_reload", _on_resources_reloaded)

func trigger_file_changed(file_path: String, content: String) -> void:
  var stored_content = file_contents.get(file_path, "")
  if content != stored_content:
    file_contents[file_path] = content
    file_changed.emit(file_path, content)

## FILE SYSTEM CHANGED

func _on_filesystem_changed():
  _scan_for_changes()

func _on_resources_reloaded(resources: Array):
  for path in resources:
      _check_file_changes(path)

func _scan_for_changes():
  var dir = DirAccess.open("res://")
  if dir:
      _scan_directory(dir, "res://")

func _scan_directory(dir: DirAccess, current_path: String):
  # Recursively scan directories for files
  dir.list_dir_begin()
  var file_name = dir.get_next()
  
  while file_name != "":
      if file_name == "." or file_name == "..":
          file_name = dir.get_next()
          continue
          
      var full_path = current_path.path_join(file_name)
      
      if dir.current_is_dir():
          var sub_dir = DirAccess.open(full_path)
          if sub_dir:
              _scan_directory(sub_dir, full_path)
      else:
          _check_file_changes(full_path)
          
      file_name = dir.get_next()

func _check_file_changes(file_path: String):
  # Skip files that aren't GDScript or scene file
  # todo: support binary files
  if not file_path.ends_with(".gd") and not file_path.ends_with(".tscn"):
    return

  var file = FileAccess.open(file_path, FileAccess.READ)
  if not file:
      return
  
  var content = file.get_as_text(true)


  trigger_file_changed(file_path, content)


## SCENE CHANGED

# todo: figure out how to do this without creating a temp file
# todo: figure out how to make ids stable
func _on_changed():
  if _ignore_changes:
    return

  var root = editor_plugin.get_editor_interface().get_edited_scene_root()
  if root:
    var packed_scene = PackedScene.new()
    packed_scene.pack(root)
    
    var temp_path = "user://scene.tscn"
    
    # Save to temp file
    var error = ResourceSaver.save(packed_scene, temp_path)
    if error != OK:
      print("Error saving scene: ", error)
      return
      
    # Read the file contents
    var file = FileAccess.open(temp_path, FileAccess.READ)
    if file:
      var content = file.get_as_text()
      trigger_file_changed(root.scene_file_path, content)
      file.close()


    # Delete the temp file
    DirAccess.remove_absolute(temp_path)

@tool
extends EditorPlugin

var file_change_listener: FileChangeListener
var automerge_fs: AutomergeFS
var sidebar


func _enter_tree() -> void:
  print("start patchwork");

  automerge_fs = AutomergeFS.create("08d79d8e432046c0b8df0e320d5edf0b")
  automerge_fs.start();

  # listen to remote changes
  automerge_fs.file_changed.connect(_on_remote_file_changed)

  # listen to local changes
  file_change_listener = FileChangeListener.new(self)
  file_change_listener.connect("file_changed", _on_local_file_changed)

  # setup sidebar
  sidebar = preload("res://addons/patchwork/godot/sidebar.tscn").instantiate()
  sidebar.init(self)
  add_control_to_dock(DOCK_SLOT_RIGHT_UL, sidebar)

func _on_local_file_changed(path: String, content: String) -> void:
  # for now ignore all files that are not main.tscn
  if not path.ends_with("main.tscn"):
    return

  print("filechanged", path);
  automerge_fs.save(path, content);

func _on_remote_file_changed(patch) -> void:
  var node_path = patch.node_path
  var file_path = patch.file_path

  var scene = get_editor_interface().get_edited_scene_root()

  if not scene:
    return
    
  if scene.scene_file_path != file_path:
    return
    
  var node = scene.get_node(node_path)
  if not node:
    return

  if patch.type == "property_changed":
    # print("changed ", node_path, " ", patch.key, " ", patch.value)

    var value = null

    if patch.value.begins_with("res://"):
      value = load(patch.value)

      if "instantiate" in value:
        value = value.instantiate()

    else:
      value = str_to_var(patch.value)
  

    if value != null:
      node.set(patch.key, value);


  # # for now ignore all files that are not main.tscn
  # if not path.ends_with("main.tscn"):
  #   return

  # # Check if file exists and get current content
  # var current_file = FileAccess.open(path, FileAccess.READ)
  # if not current_file:
  #   return
  # var current_content = current_file.get_as_text()
  # current_file.close()

  # # Skip if content hasn't changed
  # if current_content == content:
  #   return

  # var file = FileAccess.open(path, FileAccess.WRITE)
  # if not file:
  #   return
    
  # # Write the content to the file
  # file.store_string(content)
  # file.close()
  
  # print("reload path", path)

  # # Reload file
  # get_editor_interface().reload_scene_from_path(path)
  # print("remote file changed ", path)


func _process(delta: float) -> void:

  if automerge_fs:
    automerge_fs.refresh();

func _exit_tree() -> void:
  if sidebar:
    remove_control_from_docks(sidebar)

  if automerge_fs:
    automerge_fs.stop();

  if file_change_listener:
    file_change_listener.stop()

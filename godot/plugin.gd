@tool
extends EditorPlugin

var file_change_listener: FileChangeListener
var automerge_fs: AutomergeFS
var config: PatchworkConfig
var sidebar


func _enter_tree() -> void:
  print("start patchwork!!");

  # setup config
  config = PatchworkConfig.new()
  

  # setup automerge fs
  var project_url = config.get_value("project_url", "")
  automerge_fs = AutomergeFS.create(project_url)
  if !project_url:
    config.set_value("project_url", automerge_fs.get_fs_doc_id());

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

  automerge_fs.save(path, content);

func _on_remote_file_changed(patch) -> void:
  var scene = get_editor_interface().get_edited_scene_root()

  if not scene:
    return
    
  if scene.scene_file_path != patch.file_path:
    return

  # lookup node
  var node = null
  if scene.has_node(patch.node_path):
    node = scene.get_node(patch.node_path)

  # print("patch ", patch.get("type"), " ", patch.get("node_path"), " ", patch.get("key"), " ", patch.get("value"), " ", patch.get("instance_path"))
  
  # ... create node if it doesn't exist
  if not node:
    if patch.get("instance_path") || patch.get("instance_type"):
      var parent_path = patch.node_path.get_base_dir()
      var parent = scene if parent_path == "" else scene.get_node(parent_path)
      print("create based on instance_path")
      if parent:
        var instance = null
        if patch.instance_path:
          instance = load(patch.instance_path).instantiate()
        else:
          instance = ClassDB.instantiate(patch.get("instance_type"))
        
        instance.name = patch.node_path.get_file()
        parent.add_child(instance)
        instance.owner = scene
        node = instance

  if not node:
    assert(false, "invalid state - couldn't create node")
    return
  
  # PROPERTY CHANGED
  if patch.type == "property_changed":
    # print("prop changed ", patch.node_path, " ", patch.key, " ", patch.value)
    var value = null

    if patch.value.begins_with("res://"):
      value = load(patch.value)
      if "instantiate" in value:
        value = value.instantiate()

    elif patch.value.begins_with("SubResource"):
      # Ignore sub-resources for now
      pass
    else:
      value = str_to_var(patch.value)

    if value != null:
      if not is_same(node.get(patch.key), value):
        var undo_redo = get_undo_redo()
        file_change_listener.ignore_changes(func():
            undo_redo.create_action("Change " + patch.node_path.get_file() + "." + patch.key)
            undo_redo.add_do_property(node, patch.key, value)
            undo_redo.add_undo_property(node, patch.key, node.get(patch.key))
            undo_redo.commit_action()
        )

  # DELETE NODE
  elif patch.type == "node_deleted":
    if node:
      file_change_listener.ignore_changes(func():
        print("delete node ", patch.node_path)
        node.get_parent().remove_child(node)
        node.queue_free()
      )


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

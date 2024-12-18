@tool
extends EditorPlugin

var file_change_listener: FileChangeListener
var automerge_fs: AutomergeFS
var config: PatchworkConfig
var sidebar
var checked_out_branch_doc_id = null

func _enter_tree() -> void:
  print("start patchwork!!!!");

  # setup config
  config = PatchworkConfig.new()
  
  # setup automerge fs
  var project_doc_id = config.get_value("project_doc_id", "")
  automerge_fs = AutomergeFS.create(project_doc_id)
  automerge_fs.connect("branch_list_changed", _on_branch_list_changed);
  automerge_fs.connect("patch_file", _on_remote_patch_file);
  automerge_fs.connect("reload_file", _on_remote_reload_file);

  if !project_doc_id:
    config.set_value("project_doc_id", automerge_fs.get_branches_metadata_doc_id());

  automerge_fs.start();

  # listen to remote changes

  # listen to local changes
  file_change_listener = FileChangeListener.new(self)
  file_change_listener.connect("file_changed", _on_local_file_changed)
  
  # setup sidebar
  sidebar = preload("res://addons/patchwork/godot/sidebar.tscn").instantiate()
  sidebar.init(self)

  sidebar.connect("create_new_branch", _on_create_new_branch);
  sidebar.connect("checkout_branch", _on_checkout_branch);
  add_control_to_dock(DOCK_SLOT_RIGHT_UL, sidebar)


func _on_create_new_branch(branch_name: String) -> void:
  automerge_fs.create_branch(branch_name)

func _on_checkout_branch(branch_doc_id: String) -> void:
  print("checkout");
  checked_out_branch_doc_id = branch_doc_id
  automerge_fs.checkout(branch_doc_id)

func _on_branch_list_changed(branches) -> void:
  if !checked_out_branch_doc_id:
    automerge_fs.checkout(branches[0].id)

  print("update branches", branches)
  sidebar.update_branches(branches)

func _on_local_file_changed(path: String, content: String) -> void:
  # for now ignore all files that are not main.tscn
  if not path.ends_with("main.tscn"):
    return

  automerge_fs.save(path, content);

func _on_remote_reload_file(path, _content) -> void:

  print("reload file ", path)

  # Save the new content to disk
  var file = FileAccess.open(path, FileAccess.WRITE)
  file.store_string(_content)
  file.close()

  # Reload the scene in the editor
  get_editor_interface().reload_scene_from_path(path)


func _on_remote_patch_file(patch) -> void:
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


func _process(_delta: float) -> void:
  if automerge_fs:
    automerge_fs.refresh();

func _exit_tree() -> void:
  if sidebar:
    remove_control_from_docks(sidebar)

  if automerge_fs:
    automerge_fs.stop();

  if file_change_listener:
    file_change_listener.stop()

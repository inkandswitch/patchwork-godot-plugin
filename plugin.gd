@tool
extends EditorPlugin

var godot_project: GodotProject
var config: PatchworkConfig
var file_system: FileSystem
var sidebar

const TEMP_DIR = "user://tmp"

var last_synced_heads: PackedStringArray
# Array of [<path>, <content>]
var file_content_to_reload: Array = []
var files_to_reload_mutex: Mutex = Mutex.new()
var current_pw_to_godot_sync_task_id: int = -1
var deferred_pw_to_godot_sync: bool = false
var timer: SceneTreeTimer = null

var prev_checked_out_branch_id

func add_new_uid(path: String, uid: String):
	var id = ResourceUID.text_to_id(uid)
	if id == ResourceUID.INVALID_ID:
		return
	if not ResourceUID.has_id(id):
		ResourceUID.add_id(id, path)
	elif not ResourceUID.get_id_path(id) == path:
		ResourceUID.set_id(id, path)
		
func _process(_delta: float) -> void:
	if godot_project:
		godot_project.process()

func _enter_tree() -> void:
	print("start patchwork!!!");
	config = PatchworkConfig.new();
	file_system = FileSystem.new(self)

	await init_godot_project()

	# listen for file changes once we have initialized the godot project
	file_system.connect("file_changed", _on_local_file_changed)

	# setup patchwork sidebar
	sidebar = preload("res://addons/patchwork/sidebar.tscn").instantiate()
	sidebar.init(self, godot_project, config)
	add_control_to_dock(DOCK_SLOT_RIGHT_UL, sidebar)

func init_godot_project():
	print("init_godot_project()")
	var project_doc_id = config.get_project_value("project_doc_id", "")
	var user_name = config.get_user_value("user_name", "")

	godot_project = GodotProject.create(project_doc_id, user_name)

	if godot_project == null:
		print("Failed to create GodotProject instance.")
		return

	await godot_project.checked_out_branch

	print("*** Patchwork Godot Project initialized! ***")
	if !project_doc_id:
		config.set_project_value("project_doc_id", godot_project.get_project_doc_id())
		sync_godot_to_patchwork()
	else:
		sync_patchwork_to_godot()

	godot_project.connect("files_changed", func():
		print("files changed!!!!")
		sync_patchwork_to_godot()
	)
	godot_project.checked_out_branch.connect(_on_checked_out_branch)
	print("end init_godot_project()")

func sync_godot_to_patchwork():
	var files_in_godot = get_relevant_godot_files()

	print("sync godot -> patchwork (", files_in_godot.size(), ")")

	var files_to_save = {}

	for path in files_in_godot:
		files_to_save[path] = file_system.get_file(path)

	print("saved ", files_to_save.size(), " file(s) to patchwork")

	godot_project.save_files(files_to_save)

	last_synced_heads = godot_project.get_heads()

func sync_patchwork_to_godot():
	if PatchworkEditor.unsaved_files_open():
		print("unsaved files open, not syncing")
		return

	var files_in_patchwork = godot_project.list_all_files()

	var files_to_reimport = {}

	file_system.disconnect_from_file_system()

	print("sync patchwork -> godot (", files_in_patchwork.size(), ")")

	for path in files_in_patchwork:
		var patchwork_content = godot_project.get_file(path)
		var fs_content = file_system.get_file(path)

		if typeof(patchwork_content) == TYPE_NIL:
			printerr("patchwork missing file content even though path exists: ", path)
			continue

		elif fs_content != null and typeof(fs_content) != typeof(patchwork_content):
			# log if current content is not the same type as content
			printerr("different types at ", path, ": ", typeof(fs_content), " vs ", typeof(patchwork_content))
			continue

		# skip files that are already in sync
		if patchwork_content == fs_content:
			continue

		# reload after sync
		file_system.save_file(path, patchwork_content)
		if path.get_extension() == "uid":
			var new_path = path.get_basename()
			var uid = patchwork_content.strip_edges()
			add_new_uid(new_path, uid)

		if path.get_extension() == "import":
			var new_path = path.get_basename()
			files_to_reimport[new_path] = true
			var uid = ""
			for line in patchwork_content.split("\n"):
				if line.begins_with("uid="):
					uid = line.split("=")[1].strip_edges()
			add_new_uid(new_path, uid)
		elif FileAccess.file_exists(path + ".import"):
			files_to_reimport[path] = true
		
		if path.get_extension() == "tscn":
			# reload scene files to update references
			EditorInterface.reload_scene_from_path(path)

		if files_to_reimport.size() > 0:
			EditorInterface.get_resource_filesystem().reimport_files(files_to_reimport.keys())

	file_system.connect_to_file_system()

const BANNED_FILES = [".DS_Store", "thumbs.db", "desktop.ini"] # system files that should be ignored

func _is_relevant_file(path: String) -> bool:
	if path.trim_prefix("res://").begins_with("target"):
		return false
	if path.begins_with("res://patchwork.cfg") or path.begins_with("res://addons/") or path.begins_with("res://target/") or path.begins_with("res://."):
		return false

	var file = path.get_file()
	if BANNED_FILES.has(file):
		return false

	return true

func get_relevant_godot_files() -> Array[String]:
	# right now we only sync script and scene files, also we ignore the addons folder
	var ret = file_system.list_all_files().filter(_is_relevant_file)
	# print(ret)
	return ret

func _on_checked_out_branch(checked_out_branch: String):
	sync_patchwork_to_godot()
	sidebar._after_cvs_action()
	
func _on_local_file_changed(path: String, content: Variant):
	if _is_relevant_file(path):
		# todo: do save at head, but the current synced heads are wrong 
		# so we need to fix that first
		print("saving file: ", path)

		godot_project.save_file(path, content)
		last_synced_heads = godot_project.get_heads()

func _exit_tree() -> void:
	if sidebar:
		remove_control_from_docks(sidebar)

	if is_instance_valid(godot_project):
		pass
		# godot_project.shutdown();

	if file_system:
		file_system.stop()

func show_diff(hash1, hash2):
	# TODO: handle dependencies of these files
	var diff_dict = godot_project.get_changed_file_content_between([hash1], [hash2])
	var files_arr = diff_dict["files"]
	if files_arr.size() == 0:
		print("No changes between %s and %s" % [hash1, hash2])
		return
	print("Changes between %s and %s:" % [hash1, hash2])
	var new_dict = {}
	var new_files = []
	for file: Dictionary in files_arr:
		var path = file["path"]
		var change = file["change"]
		var old_content = file["old_content"]
		var new_content = file["new_content"]
		# for all the files in the dict, save as tmp files

		print("File: %s" % path)
		print("Change: %s" % change)
		var old_path = TEMP_DIR.path_join(path.trim_prefix("res://")) + "_old"
		var new_path = TEMP_DIR.path_join(path.trim_prefix("res://")) + "_new"
		if change == "added":
			old_path = null
			print("New content: %s" % new_content)
		if change == "deleted":
			new_path = null
		if old_path:
			file_system.save_file(old_path, old_content)
		if new_path:
			file_system.save_file(new_path, new_content)
		new_files.append({
			"path": path,
			"change": change,
			"old_content": old_content,
			"new_content": new_content
		})
	new_dict["files"] = new_files
	PatchworkEditor.show_diff(new_dict)

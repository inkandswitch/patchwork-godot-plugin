@tool
extends EditorPlugin

var config: PatchworkConfig
var file_system: FileSystem
var sidebar

var last_synced_heads: PackedStringArray
var task_modal: TaskModal = TaskModal.new()

func add_new_uid(path: String, uid: String):
	var id = ResourceUID.text_to_id(uid)
	if id == ResourceUID.INVALID_ID:
		return
	if not ResourceUID.has_id(id):
		ResourceUID.add_id(id, path)
	elif not ResourceUID.get_id_path(id) == path:
		ResourceUID.set_id(id, path)

func _process(_delta: float) -> void:
	pass

func _enter_tree() -> void:
	# need to add task_modal as a child to the plugin otherwise process won't be called
	add_child(task_modal)

	config = PatchworkConfig.new();
	file_system = FileSystem.new(self)

	task_modal.start_task("Loading Patchwork")

	await init_godot_project()

	task_modal.end_task("Loading Patchwork")

	print("checked out branch: ", GodotProject.get_checked_out_branch())

	# listen for file changes once we have initialized the godot project
	file_system.connect("file_changed", _on_local_file_changed)

	# setup patchwork sidebar
	sidebar = preload("res://addons/patchwork/gdscript/sidebar.tscn").instantiate()
	print("sidebar instantiated ", sidebar)

	sidebar.init(self, config)
	add_control_to_dock(DOCK_SLOT_RIGHT_UL, sidebar)


func init_godot_project():
	var storage_folder_path = ProjectSettings.globalize_path("res://.patchwork")

	print("init_godot_project()")
	var project_doc_id = config.get_project_value("project_doc_id", "")
	var checked_out_branch_doc_id = config.get_project_value("checked_out_branch_doc_id", "")
	var user_name = config.get_user_value("user_name", "")

	print("wait for checked out branch")
	var checked_out_branch = GodotProject.get_checked_out_branch()
	if checked_out_branch == null:
		print("checked out branch is null, waiting for signal...")
		await GodotProject.checked_out_branch

	config.set_project_value("checked_out_branch_doc_id", GodotProject.get_checked_out_branch().id)

	print("*** Patchwork Godot Project initialized! ***")
	if !project_doc_id:
		config.set_project_value("project_doc_id", GodotProject.get_project_doc_id())
		sync_godot_to_patchwork()
	else:
		sync_patchwork_to_godot()

	GodotProject.connect("files_changed", func():
		print("files changed!!!!")
		sync_patchwork_to_godot()
	)
	GodotProject.checked_out_branch.connect(_on_checked_out_branch)
	print("end init_godot_project()")

func sync_godot_to_patchwork():
	var files_in_godot = get_relevant_godot_files()

	print("sync godot -> patchwork (", files_in_godot.size(), ")")

	var files_to_save = {}

	for path in files_in_godot:
		files_to_save[path] = file_system.get_file(path)

	print("saved ", files_to_save.size(), " file(s) to patchwork")

	GodotProject.save_files(files_to_save)

	last_synced_heads = GodotProject.get_heads()

func sync_patchwork_to_godot():
	var start_time = Time.get_ticks_msec()

	if PatchworkEditor.unsaved_files_open():
		print("unsaved files open, not syncing")
		return

	var files_in_patchwork = GodotProject.get_files()

	var files_to_reimport = {}
	var scenes_to_reload = []
	var reload_scripts = false
	file_system.disconnect_from_file_system()

	print("sync patchwork -> godot (", files_in_patchwork.size(), ")")

	for path in files_in_patchwork:
		var patchwork_content = files_in_patchwork[path]

		if typeof(patchwork_content) == TYPE_NIL:
			printerr("patchwork missing file content even though path exists: ", path)
			continue

		var fs_content = file_system.get_file(path)

		if fs_content != null and typeof(fs_content) != typeof(patchwork_content):
			# log if current content is not the same type as content
			printerr("different types at ", path, ": ", typeof(fs_content), " vs ", typeof(patchwork_content))
			continue


		# skip files that are already in sync
		# exeption: always reload open scenes, because the scene might not have changed but a contained scene might have
		if patchwork_content == fs_content:
			continue


		print("file changed: ", path)

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
					break
			add_new_uid(new_path, uid)
		elif FileAccess.file_exists(path + ".import"):
			files_to_reimport[path] = true
		# if it's a script,
		elif path.get_extension() == "gd":
			reload_scripts = true

		if path.get_extension() == "tscn":
			# reload scene files to update references
			scenes_to_reload.append(path)

	if reload_scripts:
		PatchworkEditor.reload_scripts(false)

	if files_to_reimport.size() > 0:
		EditorInterface.get_resource_filesystem().reimport_files(files_to_reimport.keys())

	if scenes_to_reload.size() > 0:
		for scene_path in scenes_to_reload:
			EditorInterface.reload_scene_from_path(scene_path)



	file_system.connect_to_file_system()

	print("sync patchwork -> godot took ", Time.get_ticks_msec() - start_time, "ms")

const BANNED_FILES = [".DS_Store", "thumbs.db", "desktop.ini"] # system files that should be ignored

func _is_relevant_file(path: String) -> bool:
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
	config.set_project_value("checked_out_branch_doc_id", checked_out_branch)
	sync_patchwork_to_godot()

func _on_local_file_changed(path: String, content: Variant):
	if _is_relevant_file(path):
		# todo: do save at head, but the current synced heads are wrong
		# so we need to fix that first
		print("saving file: ", path)

		GodotProject.save_file(path, content)
		last_synced_heads = GodotProject.get_heads()

func _exit_tree() -> void:
	print("exit patchwork!!!")
	if sidebar:
		remove_control_from_docks(sidebar)

	# if is_instance_valid(GodotProject):
	# 	pass
		# GodotProject.shutdown();

	if file_system:
		file_system.stop()

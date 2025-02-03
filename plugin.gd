@tool
extends EditorPlugin

var godot_project: GodotProject
var config: PatchworkConfig
var file_system: FileSystem
var sidebar

var last_synced_heads: PackedStringArray
# Array of [<path>, <content>]
var file_content_to_reload: Array = []
var files_to_reload_mutex: Mutex = Mutex.new()
var current_pw_to_godot_sync_task_id: int = -1
var deferred_pw_to_godot_sync: bool = false

var prev_checked_out_branch_id

func _process(_delta: float) -> void:
	if godot_project:
		godot_project.process()

	if current_pw_to_godot_sync_task_id != -1:
		# currently running sync task
		_try_wait_for_pw_to_godot_sync_task()
	else:
		# reload after sync
		var new_files_to_reload: Array = []
		files_to_reload_mutex.lock()
		# append array here, because otherwise new_files_to_reload just gets a reference to file_content_to_reload
		new_files_to_reload.append_array(file_content_to_reload)
		file_content_to_reload.clear()
		files_to_reload_mutex.unlock()
		if len(new_files_to_reload) > 0:
			print("reloading %d files: " % new_files_to_reload.size())
		for token in new_files_to_reload:
			var path = token[0]
			file_system.save_file(path, token[1])
			if path.get_extension() == "tscn":
				# reload scene files to update references
				get_editor_interface().reload_scene_from_path(path)

func _enter_tree() -> void:
	print("start patchwork!!!");

	config = PatchworkConfig.new();

	file_system = FileSystem.new(self)
	
	print("_enter_tree() -> init_godot_project()")
	init_godot_project()
	print("end _enter_tree() -> init_godot_project()")

	# listen for file changes once we have initialized the godot project
	file_system.connect("file_changed", _on_local_file_changed)
	
	# setup patchwork sidebar
	sidebar = preload("res://addons/patchwork/sidebar.tscn").instantiate()
	sidebar.init(self, godot_project)
	add_control_to_dock(DOCK_SLOT_RIGHT_UL, sidebar)

func init_godot_project():
	print("init_godot_project()")
	var project_doc_id = config.get_value("project_doc_id", "")


	godot_project = GodotProject.create(project_doc_id)
	if godot_project == null:
		print("Failed to create GodotProject instance.")
		return


	print("*** Patchwork Godot Project initialized! ***")
	if !project_doc_id:
		config.set_value("project_doc_id", godot_project.get_doc_id())
		sync_godot_to_patchwork()
	else:
		sync_patchwork_to_godot()

	godot_project.connect("files_changed", sync_patchwork_to_godot)
	godot_project.checked_out_branch.connect(_on_checked_out_branch)
	print("end init_godot_project()")

func do_sync_godot_to_patchwork():
	var files_in_godot = get_relevant_godot_files()

	print("sync godot -> patchwork (", files_in_godot.size(), ")")

	for path in files_in_godot:
		print("  save file: ", path)
		godot_project.save_file(path, file_system.get_file(path))


func sync_godot_to_patchwork():
	# TODO: this is synchronous for right now because GodotProject doesn't seem to be thread safe currently, getting deadlocks
	# We need to wait for any pw_to_godot sync to finish before we start syncing in the other direction
	_try_wait_for_pw_to_godot_sync_task(true)
	do_sync_godot_to_patchwork()
	last_synced_heads = godot_project.get_heads()


func _do_pw_to_godot_sync_element(i: int, files_in_patchwork: PackedStringArray):
	if i >= files_in_patchwork.size():
		return

	var path = files_in_patchwork[i]
	var gp_content = godot_project.get_file(path)
	var fs_content = file_system.get_file(path)

	if typeof(gp_content) == TYPE_NIL:
		printerr("patchwork missing file content even though path exists: ", path)
		return
	elif fs_content != null and typeof(fs_content) != typeof(gp_content):
		# log if current content is not the same type as content
		printerr("different types at ", path, ": ", typeof(fs_content), " vs ", typeof(gp_content))
		return

	if gp_content != fs_content:
		print("  reload file: ", path)
		# The reason why we're not simply reloading here is that loading resources gets kinda dicey on anything other than the main thread
		files_to_reload_mutex.lock()
		file_content_to_reload.append([path, gp_content])
		files_to_reload_mutex.unlock()


func do_pw_to_godot_sync_task():
	print("performing patchwork to godot sync in parallel...")

	var files_in_godot = get_relevant_godot_files()
	var files_in_patchwork = godot_project.list_all_files()

	print("sync patchwork -> godot (", files_in_patchwork.size(), ")")

	var group_id = WorkerThreadPool.add_group_task(self._do_pw_to_godot_sync_element.bind(files_in_patchwork), files_in_patchwork.size())
	WorkerThreadPool.wait_for_group_task_completion(group_id)

	# todo: this is still buggy
	# delete gd and tscn files that are not in checked out patchwork files
	# for path in files_in_godot:
	# 	if !files_in_patchwork.has(path) and (path.ends_with(".gd") or path.ends_with(".tscn")):
	# 		print("  delete file: ", path)
	# 		file_system.delete_file(path)

	print("end patchwork to godot sync")

func _try_wait_for_pw_to_godot_sync_task(force: bool = false):
	# We have to wait for a task to complete before the program exits so we don't have zombie threads, 
	# but we don't want to block waiting for the task to complete;
	# so right now, we just check if it's completed and if not, we return immediately; 
	# _process calls this, so it will keep getting called each frame until we actually finish.
	if current_pw_to_godot_sync_task_id != -1:
		if force or WorkerThreadPool.is_task_completed(current_pw_to_godot_sync_task_id):
			WorkerThreadPool.wait_for_task_completion(current_pw_to_godot_sync_task_id)
			current_pw_to_godot_sync_task_id = -1
			last_synced_heads = godot_project.get_heads()

func sync_patchwork_to_godot():
	# only sync once the user has saved all files
	if godot_project.unsaved_files_open():
		return
	current_pw_to_godot_sync_task_id = WorkerThreadPool.add_task(self.do_pw_to_godot_sync_task, false, "sync_patchwork_to_godot")
	call_deferred("_try_wait_for_pw_to_godot_sync_task")

	return

const BANNED_FILES = [".DS_Store", "thumbs.db", "desktop.ini"] # system files that should be ignored

func _is_relevant_file(path: String) -> bool:
	var is_excluded_path = path.begins_with("res://addons/") or path.begins_with("res://target/")
	if is_excluded_path:
		return false

	var file = path.get_file()
	if BANNED_FILES.has(file):
		return false

	return true

func get_relevant_godot_files() -> Array[String]:
	# right now we only sync script and scene files, also we ignore the addons folder
	return file_system.list_all_files().filter(_is_relevant_file)

func _on_checked_out_branch(checked_out_branch: String):
	print("checked out branch ", checked_out_branch, " (", godot_project.list_all_files().size(), " files)")

	sidebar.update_ui(checked_out_branch)
	sync_patchwork_to_godot()
	
func _on_local_file_changed(path: String, content: Variant):
	print("file changed", path)

	if _is_relevant_file(path):
		print("save file: ", path)

		var heads_string = ",".join(Array(last_synced_heads))

		godot_project.save_file_at(path, heads_string, content)
		last_synced_heads = godot_project.get_heads()


func _exit_tree() -> void:
	_try_wait_for_pw_to_godot_sync_task(true)
	if sidebar:
		remove_control_from_docks(sidebar)

	if godot_project:
		godot_project.stop();

	if file_system:
		file_system.stop()

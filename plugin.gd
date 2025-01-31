@tool
extends EditorPlugin

var godot_project: GodotProject
var config: PatchworkConfig
var file_system: FileSystem
var sidebar

var last_synced_heads: PackedStringArray

func _process(_delta: float) -> void:
	if godot_project:
		godot_project.process()

func _enter_tree() -> void:
	print("start patchwork!!!");

	config = PatchworkConfig.new();

	file_system = FileSystem.new(self)
	
	print("_enter_tree() -> init_godot_project()")
	await init_godot_project()
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


	await godot_project.initialized

	print("*** Patchwork Godot Project initialized! ***")
	if !project_doc_id:
		config.set_value("project_doc_id", godot_project.get_doc_id())
		sync_godot_to_patchwork()
	else:
		sync_patchwork_to_godot()

	godot_project.connect("files_changed", sync_patchwork_to_godot)
	godot_project.checked_out_branch.connect(_on_checked_out_branch)
	print("end init_godot_project()")

func sync_godot_to_patchwork():
	var files_in_godot = get_relevant_godot_files()

	print("sync godot -> patchwork (", files_in_godot.size(), ")")

	for path in files_in_godot:
		print("  save file: ", path)
		godot_project.save_file(path, file_system.get_file(path))

	last_synced_heads = godot_project.get_heads()


func sync_patchwork_to_godot():
	
	# only sync once the user has saved all files
	if godot_project.unsaved_files_open():
		return

	var files_in_godot = get_relevant_godot_files()
	var files_in_patchwork = godot_project.list_all_files()

	print("sync patchwork -> godot (", files_in_patchwork.size(), ")")

	# load checked out patchwork files into godot
	for path in files_in_patchwork:
		var gp_content = godot_project.get_file(path)
		var fs_content = file_system.get_file(path)

		print("? check file: ", path)
		if typeof(gp_content) == TYPE_NIL:
			print("!!!!!ERROR: patchwork missing file content even though path exists: ", path)
			continue
		elif fs_content != null and typeof(fs_content) != typeof(gp_content):
			# log if current content is not the same type as content
			print("ERROR: different types at ", path, ": ", typeof(fs_content), " vs ", typeof(gp_content))
			continue

		if gp_content != fs_content:
			print("  reload file: ", path)
			file_system.save_file(path, gp_content)

			# Trigger reload of scene files to update references
			if path.ends_with(".tscn"):
				get_editor_interface().reload_scene_from_path(path)

	# todo: this is still buggy
	# delete gd and tscn files that are not in checked out patchwork files
	# for path in files_in_godot:
	# 	if !files_in_patchwork.has(path) and (path.ends_with(".gd") or path.ends_with(".tscn")):
	# 		print("  delete file: ", path)
	# 		file_system.delete_file(path)

	last_synced_heads = godot_project.get_heads()


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

func _on_checked_out_branch():
	print("checked out branch ", godot_project.get_branch_doc_id(), " (", godot_project.list_all_files().size(), " files)")

	sync_patchwork_to_godot()
	
func _on_local_file_changed(path: String, content: Variant):
	print("file changed", path)

	if _is_relevant_file(path):
		print("save file: ", path)

		var heads_string = ",".join(Array(last_synced_heads))

		godot_project.save_file_at(path, heads_string, content)
		last_synced_heads = godot_project.get_heads()


func _exit_tree() -> void:
	if sidebar:
		remove_control_from_docks(sidebar)

	if godot_project:
		godot_project.stop();

	if file_system:
		file_system.stop()

@tool
extends EditorPlugin

var godot_project: GodotProject
var config: PatchworkConfig
var file_system: FileSystem
var sidebar

func _process(_delta: float) -> void:
	if godot_project:
		godot_project.process()

func _enter_tree() -> void:
	print("start patchwork");

	config = PatchworkConfig.new();

	file_system = FileSystem.new(self)
	
	await init_godot_project()

	# listen for file changes once we have initialized the godot project
	# file_system.connect("file_changed", _on_local_file_changed)

	print("save file")
	var flag_content = FileAccess.get_file_as_bytes("res://assets/flag.png")
	godot_project.save_file("res://assets/flag.png", flag_content)
	var main_content = FileAccess.get_file_as_string("res://main.tscn")
	godot_project.save_file("res://main.tscn", main_content)


	print("main ", godot_project.get_file("res://main.tscn").substr(0, 50))
	print("flag ", godot_project.get_file("res://assets/flag.png"))
	
	# setup patchwork sidebarp
	# sidebar = preload("res://addons/patchwork/godot/sidebar.tscn").instantiate()
	# sidebar.init(self, godot_project)
	# add_control_to_dock(DOCK_SLOT_RIGHT_UL, sidebar)

func init_godot_project():
	var project_doc_id = "" # config.get_value("project_doc_id", "")

	godot_project = GodotProject.create(project_doc_id)

	# todo: godo project should signal when it's ready
	# right now we just wait a bit
	await get_tree().create_timer(1.0).timeout

	# if !project_doc_id:
	# 	config.set_value("project_doc_id", godot_project.get_doc_id())
	# 	sync_godot_to_patchwork()
	# else:
	# 	sync_patchwork_to_godot()

	# godot_project.connect("files_changed", sync_patchwork_to_godot)
	# godot_project.checked_out_branch.connect(_on_checked_out_branch)


func sync_godot_to_patchwork():
	var files_in_godot = get_relevant_godot_files()

	print("sync godot -> patchwork (", files_in_godot.size(), ")")

	for path in files_in_godot:
		print("  save file: ", path)
		godot_project.save_file(path, file_system.get_file(path))


func sync_patchwork_to_godot():
	
	var files_in_godot = get_relevant_godot_files()
	var files_in_patchwork = godot_project.list_all_files()

	print("sync patchwork -> godot (", files_in_patchwork.size(), ")")

	# load checked out patchwork files into godot
	for path in files_in_patchwork:
		var content = godot_project.get_file(path)
		var current_content = file_system.get_file(path)
		if content != current_content:
			print("  reload file: ", path)
			file_system.save_file(path, content)

			# Trigger reload of scene files to update references
			if path.ends_with(".tscn"):
				get_editor_interface().reload_scene_from_path(path)

	# todo: this is still buggy
	# delete gd and tscn files that are not in checked out patchwork files
	# for path in files_in_godot:
	# 	if !files_in_patchwork.has(path) and (path.ends_with(".gd") or path.ends_with(".tscn")):
	# 		print("  delete file: ", path)
	# 		file_system.delete_file(path)


func _is_relevant_file(path: String) -> bool:
	if path.begins_with("res://addons/"): return false
	return path.ends_with(".gd") or path.ends_with(".tscn")

func get_relevant_godot_files() -> Array[String]:
	# right now we only sync script and scene files, also we ignore the addons folder
	return file_system.list_all_files().filter(_is_relevant_file)

func _on_checked_out_branch(branch_id: String):
	print("checked out branch ", branch_id, " (", godot_project.list_all_files().size(), " files)")


	sync_patchwork_to_godot()
	
func _on_local_file_changed(path: String, content: String):
	if _is_relevant_file(path):
		print("save file: ", path)
		godot_project.save_file(path, content)


func _exit_tree() -> void:
	if sidebar:
		remove_control_from_docks(sidebar)

	if godot_project:
		godot_project.stop();

	if file_system:
		file_system.stop()

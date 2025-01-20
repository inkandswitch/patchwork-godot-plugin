@tool
extends EditorPlugin

var godot_project: GodotProject
var config: PatchworkConfig
var file_system: FileSystem

var is_initialized = false

func _enter_tree() -> void:
	print("start patchwork");


	config = PatchworkConfig.new();

	file_system = FileSystem.new(self)
	# file_system.connect("file_changed", _on_local_file_changed)

	init_godot_project()

	# run_test()


func init_godot_project_test():

	var project_doc_id = config.get_value("project_doc_id", "")

	godot_project = GodotProject.create(project_doc_id)

	await get_tree().create_timer(1.0).timeout

	if !project_doc_id:
		config.set_value("project_doc_id", godot_project.get_doc_id());
		godot_project.save_file("foo", "test")

	print("file!!!", godot_project.get_file("foo"))


func init_godot_project():
	var project_doc_id = config.get_value("project_doc_id", "")

	godot_project = GodotProject.create(project_doc_id)

	# todo: godo project should signal when it's ready
	# right now we just wait a bit
	await get_tree().create_timer(1.0).timeout

	# right now we only sync script and scene files, also we ignore the addons folder
	var files_in_godot = file_system.list_all_files().filter(func(path: String) -> bool:
		if path.begins_with("res://addons/"): return false
		return path.ends_with(".gd") or path.ends_with(".tscn")
	)
	
	if !project_doc_id:
		config.set_value("project_doc_id", godot_project.get_doc_id());

		print("sync godot -> patchwork")

		for path in files_in_godot:
			print("  save file: ", path)
			godot_project.save_file(path, file_system.get_file(path))

	else:

		
		print("sync patchwork -> godot")

		var files_in_patchwork = godot_project.list_all_files()

		for path in files_in_patchwork:
			print("  ", path)

		print("sync patchwork -> godot")

		# load checked out patchwork files into godot
		for path in files_in_patchwork:
			var content = godot_project.get_file(path)
			var current_content = file_system.get_file(path)
			if content != current_content:
				print("  reload file: ", path)
				file_system.save_file(path, content)

		# delete gd and tscn files that are not in checked out patchwork files
		for path in files_in_godot:
			if !files_in_patchwork.has(path) and (path.ends_with(".gd") or path.ends_with(".tscn")):
				print("  delete file: ", path)
				file_system.delete_file(path)


	is_initialized = true

			
func _on_local_file_changed(path: String, content: String):
	if is_initialized:
		godot_project.save_file(path, content)


func _exit_tree() -> void:
	# if sidebar:
	# 	remove_control_from_docks(sidebar)

	if godot_project:
		godot_project.stop();

	if file_system:
		file_system.stop()

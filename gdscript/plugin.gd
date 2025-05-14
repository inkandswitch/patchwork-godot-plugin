@tool
extends EditorPlugin

var sidebar

var last_synced_heads: PackedStringArray
var task_modal: TaskModal = TaskModal.new()

func _process(_delta: float) -> void:
	pass

func _enter_tree() -> void:
	# need to add task_modal as a child to the plugin otherwise process won't be called
	add_child(task_modal)

	task_modal.start_task("Loading Patchwork")

	await init_godot_project()

	task_modal.end_task("Loading Patchwork")

	print("checked out branch: ", GodotProject.get_checked_out_branch())

	# listen for file changes once we have initialized the godot project
	file_system.connect("file_changed", _on_local_file_changed)

	# setup patchwork sidebar
	sidebar = preload("res://addons/patchwork/gdscript/sidebar.tscn").instantiate()
	print("sidebar instantiated ", sidebar)

	sidebar.init()
	add_control_to_dock(DOCK_SLOT_RIGHT_UL, sidebar)


func init_godot_project():
	var storage_folder_path = ProjectSettings.globalize_path("res://.patchwork")

	print("init_godot_project()")
	print("wait for checked out branch")
	var checked_out_branch = GodotProject.get_checked_out_branch()
	if checked_out_branch == null:
		print("checked out branch is null, waiting for signal...")
		await GodotProject.checked_out_branch

	PatchworkConfig.set_project_value("checked_out_branch_doc_id", GodotProject.get_checked_out_branch().id)
	GodotProject.checked_out_branch.connect(_on_checked_out_branch)
	print("end init_godot_project()")

func _on_checked_out_branch(checked_out_branch: String):
	PatchworkConfig.set_project_value("checked_out_branch_doc_id", checked_out_branch)

func _exit_tree() -> void:
	print("exit patchwork!!!")
	if sidebar:
		remove_control_from_docks(sidebar)

	# if is_instance_valid(GodotProject):
	# 	pass
		# GodotProject.shutdown();

	if file_system:
		file_system.stop()

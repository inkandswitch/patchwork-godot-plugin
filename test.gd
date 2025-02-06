@tool
extends EditorPlugin

var godot_project: GodotProject


func _process(_delta: float) -> void:
	if godot_project:
		godot_project.process()


func _enter_tree() -> void:
	# var project_doc_id = "Er2op2b6hHpwFwDdzEw5FcizoQe"

	godot_project = GodotProject.create("")

	print("do stuff");

	godot_project.connect("initialized", self._on_loaded)
	
func _on_loaded() -> void:
	print("loaded project: ", godot_project.get_checked_out_branch_id())

	godot_project.save_file("res://test.txt", "test on main")

	await get_tree().create_timer(1.0).timeout

	print("content", godot_project.get_file("res://test.txt"))

	print("files on main: ", godot_project.list_all_files())

	# print("branches: ", godot_project.get_branches())

	# var main_branch_id = godot_project.get_checked_out_branch_id()

	# print("checked out branch ", main_branch_id)

	# print("create_branch 'another'");

	# godot_project.create_branch("another")

	# print("waiting for checked out branch")

	# await godot_project.checked_out_branch

	# print("done waiting for checked out branch")

	# print("branches: ", godot_project.get_branches())

	# print("checked out branch ", godot_project.get_checked_out_branch_id())

	# godot_project.save_file("res://test.txt", "on another branch")

	# print("files on another branch: ", godot_project.list_all_files())

	# print("text.txt on another branch: ", godot_project.get_file("res://test.txt"))


	# print("go back to main")

	# godot_project.checkout_branch(main_branch_id)

	# print("waiting for checked out branch")

	# await godot_project.checked_out_branch

	# print("done waiting for checked out branch")

	# print("text.txt on main: ", godot_project.get_file("res://test.txt"))

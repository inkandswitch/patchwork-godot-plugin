@tool
extends EditorPlugin

var godot_project: GodotProject


func _process(_delta: float) -> void:
	if godot_project:
		godot_project.process()


func _enter_tree() -> void:
	var project_doc_id = "Er2op2b6hHpwFwDdzEw5FcizoQe"

	godot_project = GodotProject.create(project_doc_id)

	godot_project.connect("branches_changed", self._on_branches_updated)
	godot_project.connect("checked_out_branch", self._on_branch_checked_out)


func _on_branches_updated() -> void:
	print("branches updated:")

	var branches = godot_project.get_branches()
	for branch in branches:
		print("  ", branch.name)

	print("checkout", branches[0])

	godot_project.checkout_branch(branches[0].id)


func _on_branch_checked_out(branch_id: String) -> void:
	print("branch checked out: ", branch_id)

	for path in godot_project.list_all_files():
		print("file:", path)

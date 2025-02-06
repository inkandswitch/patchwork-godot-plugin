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


func _on_branches_updated() -> void:
	print("branches updated")

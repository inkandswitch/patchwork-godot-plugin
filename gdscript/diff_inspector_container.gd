@tool
class_name DiffInspectorContainer
extends ScrollContainer

@export var added_icon: Texture2D
@export var removed_icon: Texture2D
@export var modified_icon: Texture2D

@export var added_color: Color
@export var removed_color: Color
@export var modified_color: Color


@onready var main_vbox: VBoxContainer = %DifferMainVBox
var diff_result: DiffResult

var categories: Array = []	
var sections: Array = []

func _ready() -> void:
	pass


# Called every frame. 'delta' is the elapsed time since the previous frame.
func _process(delta: float) -> void:
	pass

func _on_button_pressed() -> void:
	pass

func update_property_editor(editor_property) -> void:
	editor_property.set_read_only(true)
	editor_property.update_property()
	editor_property.update_editor_property_status()
	editor_property.update_cache()

func getDeletedNodes() -> Array[Node]:
	var deleted_nodes: Array[Node] = []
	for section in sections:
		if section.get_object() is Node && section.get_object().get_object_name() == "Deleted Node":
			deleted_nodes.append(section.get_object())
	return deleted_nodes

func getAddedNodes() -> Array[Node]:
	var added_nodes: Array[Node] = []
	for section in sections:
		if section.get_object() is Node && section.get_object().get_object_name() == "Added Node":
			added_nodes.append(section.get_object())
	return added_nodes

func getChangedNodes() -> Array[Node]:
	var changed_nodes: Array[Node] = []
	for section in sections:
		#check if it is a node tho
		if section.get_object() is Node:
		changed_nodes.append(section.get_object())
	return changed_nodes

func add_PropertyDiffResult(editor_vbox: Control, property_diff: PropertyDiffResult) -> void:
	if property_diff == null || property_diff.get_change_type() != "changed":
		return
	var prop_name = property_diff.get_name()
	var prop_type = property_diff.get_change_type()
	var prop_old = property_diff.get_old_value()
	var prop_new = property_diff.get_new_value()	
	var prop_old_object = property_diff.get_old_object()
	var prop_new_object = property_diff.get_new_object()
	print("Adding property diff result for ", prop_name, " with type ", prop_type)
	var editor_property_new: DiffInspectorProperty = DiffInspector.instantiate_property_editor(prop_new_object, prop_name, false)
	var editor_property_old: DiffInspectorProperty = DiffInspector.instantiate_property_editor(prop_old_object, prop_name, false)
	editor_property_new.set_object_and_property(prop_new_object, prop_name)
	editor_property_old.set_object_and_property(prop_old_object, prop_name)
	update_property_editor(editor_property_new)
	update_property_editor(editor_property_old)
	editor_vbox.add_child(editor_property_new)
	editor_vbox.add_child(editor_property_old)


func add_ObjectDiffResult(object_diff: ObjectDiffResult) -> void:
	var prop_results: Array[PropertyDiffResult] = []
	var prop_diffs: Dictionary = object_diff.get_property_diffs()
	for prop in prop_diffs.keys():
		var prop_diff: PropertyDiffResult = prop_diffs[prop]
		prop_results.append(prop_diff)
	var object_name = object_diff.get_name()
	var prop_old_object = object_diff.get_old_object()
	var prop_new_object = object_diff.get_new_object()
	var object = prop_new_object if prop_new_object != null else prop_old_object
	if object == null:
		print("object is null!!!!!!!!!!!!")
		return
	var inspector_section: EditorInspectorSection = EditorInspectorSection.new()
	inspector_section.setup(object_name, object_name, object, added_color, true)
	var vbox = inspector_section.get_vbox()
	for prop_result in prop_results:
		add_PropertyDiffResult(vbox, prop_result)
	sections.append(inspector_section)
	main_vbox.add_child(inspector_section)

func add_NodeDiffResult(node_diff: NodeDiffResult) -> void:
	var node_name: String = node_diff.get_path()
	var node_type: String = node_diff.get_type()
	var node_old_object = node_diff.get_old_object()
	var node_new_object = node_diff.get_new_object()
	if node_old_object == null && node_new_object == null:
		print("node_old_object and node_new_object are null!!!!!!!!!!!!")
		return
	# the only difference between this and add_ObjectDiffResult is that we check if it's an added node or removed node
	# and we don't add the old object to the inspector
	var inspector_section: EditorInspectorSection = EditorInspectorSection.new()
	var vbox = inspector_section.get_vbox()
	if node_type == "node_added":
		inspector_section.setup(node_name, node_name, node_new_object, added_color, true)
	elif node_type == "node_deleted":
		inspector_section.setup(node_name, node_name, node_old_object, removed_color, true)
	else:
		inspector_section.setup(node_name, node_name, node_new_object, modified_color, true)
		var prop_results: Array[PropertyDiffResult] = []
		var prop_diffs: ObjectDiffResult = node_diff.get_props()
		var prop_diffs_dict: Dictionary = prop_diffs.get_property_diffs()
		for prop in prop_diffs_dict.keys():
			var prop_diff: PropertyDiffResult = prop_diffs_dict[prop]
			add_PropertyDiffResult(vbox, prop_diff)
	sections.append(inspector_section)
	main_vbox.add_child(inspector_section)


func add_FileDiffResult(file_path: String, file_diff: FileDiffResult) -> void:
	if !is_instance_valid(file_diff):
		return
	var file_name = file_path
	var type = file_diff.get_type()
	var object_results: Array[ObjectDiffResult] = []
	var node_diffs: Dictionary = {}
	if type == "resource_changed":
		var res_old = file_diff.get_res_old()
		var res_new = file_diff.get_res_new()
		var props: ObjectDiffResult = file_diff.get_props()
		if props.get_property_diffs().size() > 0:
			add_ObjectDiffResult(props)
	elif type == "scene_changed":
		node_diffs = file_diff.get_node_diffs()
		for node in node_diffs.keys():
			var node_diff: NodeDiffResult = node_diffs[node]
			add_NodeDiffResult(node_diff)
			
# defs for these are in editor/diff_result.h
func add_diff(diff: DiffResult) -> void:
	reset()
	diff_result = diff
	var file_diffs: Dictionary = diff.get_file_diffs()
	for file in file_diffs.keys():
		add_FileDiffResult(file, file_diffs[file])

	
func reset() -> void:
	for section in sections:
		section.queue_free()
	sections.clear()
	categories.clear()
	for child in main_vbox.get_children():
		child.queue_free()
	
	
func get_main_vbox() -> VBoxContainer:
	return main_vbox

func _init():
	pass

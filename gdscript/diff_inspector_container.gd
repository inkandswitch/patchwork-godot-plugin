@tool
class_name DiffInspectorContainer
extends ScrollContainer

@export var added_icon: Texture2D
@export var removed_icon: Texture2D
@export var modified_icon: Texture2D

@export var added_color: Color
@export var removed_color: Color
@export var modified_color: Color

var diff_stylebox_tex = preload("./diff_stylebox_tex.png")
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


func get_diff_stylebox(color: Color) -> StyleBoxTexture:
	var stylebox: StyleBoxTexture = StyleBoxTexture.new()
	stylebox.texture = diff_stylebox_tex
	stylebox.modulate_color = color
	return stylebox

func get_added_stylebox() -> StyleBoxTexture:
	return get_diff_stylebox(added_color)

func get_removed_stylebox() -> StyleBoxTexture:
	return get_diff_stylebox(removed_color)

func get_modified_stylebox() -> StyleBoxTexture:
	return get_diff_stylebox(modified_color)

func add_color_marker(color: Color, panel_container: PanelContainer) -> void:
	var color_rect: ColorRect = ColorRect.new()
	color_rect.color = color
	color_rect.custom_minimum_size = Vector2(10, 10)
	color_rect.layout_direction = 2 # horizontal
	color_rect.layout_mode = 2 # manual
	color_rect.size_flags_horizontal = 4 # expand

	var margin_container: MarginContainer = MarginContainer.new()
	margin_container.layout_mode = 2 # manual
	margin_container.add_theme_constant_override("margin_right", 20)
	margin_container.add_child(color_rect)

	panel_container.add_child(margin_container)

func add_label(label: String, panel_container: PanelContainer) -> void:
	var label_node: Label = Label.new()
	label_node.text = label
	panel_container.add_child(label_node)

func snake_case_to_human_readable(snake_case_string: String) -> String:
	var words = snake_case_string.split("_")
	var title_case_words = []
	for word in words:
		if word.length() > 0:
			title_case_words.append(word[0].to_upper() + word.substr(1))
	return " ".join(title_case_words)

func add_PropertyDiffResult(editor_vbox: Control, property_diff: PropertyDiffResult) -> void:
	var has_prop_new = true
	var has_prop_old = true
	if property_diff == null:
		return
	if property_diff.get_change_type() == "added":
		has_prop_old = false
	if property_diff.get_change_type() == "removed":
		has_prop_new = false
	var prop_name = property_diff.get_name()
	var prop_label = snake_case_to_human_readable(property_diff.get_name())
	var prop_type = property_diff.get_change_type()
	var prop_old = property_diff.get_old_value()
	var prop_new = property_diff.get_new_value()
	var prop_old_object = property_diff.get_old_object()
	var prop_new_object = property_diff.get_new_object()
	print("Adding property diff result for ", prop_name, " with type ", prop_type)
	var editor_property_old: DiffInspectorProperty = null
	if has_prop_old:
		editor_property_old = DiffInspector.instantiate_property_editor(prop_old_object, prop_name, false)
		editor_property_old.set_object_and_property(prop_old_object, prop_name)
		update_property_editor(editor_property_old)
		var removed_panel_container: PanelContainer = PanelContainer.new()
		add_label(prop_label, removed_panel_container)
		add_color_marker(removed_color, removed_panel_container)
		removed_panel_container.add_child(editor_property_old)
		editor_vbox.add_child(removed_panel_container)

	var editor_property_new: DiffInspectorProperty = null
	if has_prop_new:
		editor_property_new = DiffInspector.instantiate_property_editor(prop_new_object, prop_name, false)
		editor_property_new.set_object_and_property(prop_new_object, prop_name)

		var added_panel_container: PanelContainer = PanelContainer.new()

		# don't show label twice if both old and new are present
		if !has_prop_old:
			add_label(prop_label, added_panel_container)
		update_property_editor(editor_property_new)
		add_color_marker(added_color, added_panel_container)
		added_panel_container.add_child(editor_property_new)
		editor_vbox.add_child(added_panel_container)

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

	for i in range(prop_results.size()):
		if i > 0:
			var divider = HSeparator.new()
			vbox.add_child(divider)
		var prop_result = prop_results[i]
		add_PropertyDiffResult(vbox, prop_result)
		
	sections.append(inspector_section)
	main_vbox.add_child(inspector_section)

func get_flat_stylebox(color: Color) -> StyleBoxFlat:
	var stylebox: StyleBoxFlat = StyleBoxFlat.new()
	stylebox.bg_color = color
	return stylebox

func get_node_box(icon: Texture2D, text: String) -> PanelContainer:
	var panel_container: PanelContainer = PanelContainer.new()
	# HBox with two items: The removed icon and a label with the text "Node Deleted"
	var hbox: HBoxContainer = HBoxContainer.new()
	var icon_rect: TextureRect = TextureRect.new()
	icon_rect.texture = icon
	icon_rect.size.x = 60
	icon_rect.size.y = 60
	icon_rect.expand_mode = TextureRect.EXPAND_FIT_WIDTH_PROPORTIONAL
	hbox.add_child(icon_rect)
	var label: Label = Label.new()
	label.text = text
	hbox.add_child(label)
	panel_container.add_child(hbox)
	panel_container.size.y = 60
	return panel_container

func get_node_deleted_box() -> PanelContainer:
	return get_node_box(removed_icon, "Node Deleted")

func get_node_added_box() -> PanelContainer:
	return get_node_box(added_icon, "Node Added")


func add_NodeDiffResult(node_diff: NodeDiffResult) -> void:
	var node_name: String = str(node_diff.get_path()).substr(2) # remove the leading "./"
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
	print("!!! adding node diff result for ", node_name, " with type ", node_type)
	if node_type == "node_added":
		inspector_section.setup(node_name, node_name, node_new_object, added_color, true)
		print("adding node added box")
		vbox.add_child(get_node_added_box())
	elif node_type == "node_deleted":
		print("adding node deleted box")
		inspector_section.setup(node_name, node_name, node_old_object, removed_color, true)
		vbox.add_child(get_node_deleted_box())
	else:
		inspector_section.setup(node_name, node_name, node_new_object, modified_color, true)
		var prop_results: Array[PropertyDiffResult] = []
		var prop_diffs: ObjectDiffResult = node_diff.get_props()
		var prop_diffs_dict: Dictionary = prop_diffs.get_property_diffs()

		var i = 0
		for prop in prop_diffs_dict.keys():
			var prop_diff: PropertyDiffResult = prop_diffs_dict[prop]
			if i > 0:
				var divider = HSeparator.new()
				vbox.add_child(divider)
			add_PropertyDiffResult(vbox, prop_diff)
			i += 1

			
	inspector_section.unfold()
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
		print("node_diff size: ", node_diffs.size())
		for node in node_diffs.keys():
			# skip temporary nodes created by the instance
			if (String(node).contains("@")):
				continue
			var node_diff: NodeDiffResult = node_diffs[node]
			add_NodeDiffResult(node_diff)
			
# defs for these are in editor/diff_result.h
func add_diff(diff: DiffResult) -> void:
	print("ADDING DIFF!!!")
	reset()
	diff_result = diff
	var file_diffs: Dictionary = diff.get_file_diffs()
	var size = file_diffs.size()
	print("Diff size: ", size)
	for file in file_diffs.keys():
		print("Adding file diff result for ", file)
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

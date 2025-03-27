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
var diff_result: Dictionary

var categories: Array = []
var sections: Array = []
var changed_nodes: Array = []
var added_nodes: Array = []
var deleted_nodes: Array = []
var changed_resources: Array = []

func _ready() -> void:
	pass


# Called every frame. 'delta' is the elapsed time since the previous frame.
func _process(delta: float) -> void:
	pass

func _on_button_pressed() -> void:
	pass

# no type annotation for this because editor_property is ambiguously typed
func update_property_editor(editor_property) -> void:
	editor_property.set_read_only(true)
	editor_property.update_property()
	editor_property.update_editor_property_status()
	editor_property.update_cache()

func getDeletedNodes() -> Array:
	return deleted_nodes

func getAddedNodes() -> Array:
	return added_nodes

func getChangedNodes() -> Array:
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


func get_prop_editor(fake_object: MissingResource, prop_name: String, prop_value: Variant, color: Color, prop_label: String) -> PanelContainer:
	print("!!! getting prop editor for ", prop_name, " with value ", prop_value)
	fake_object.recording_properties = true
	fake_object.set(prop_name, prop_value)
	fake_object.recording_properties = false
	print("!!! fake_object prop value: ", fake_object.get(prop_name))
	if prop_label == null:
		prop_label = snake_case_to_human_readable(prop_name)
	var editor_property: DiffInspectorProperty = DiffInspector.instantiate_property_editor(fake_object, prop_name, false)
	editor_property.set_object_and_property(fake_object, prop_name)
	update_property_editor(editor_property)
	var panel_container: PanelContainer = PanelContainer.new()
	add_label(prop_label, panel_container)
	add_color_marker(color, panel_container)
	panel_container.add_child(editor_property)
	return panel_container

func add_old_and_new(inspector_section: DiffInspectorSection, change_type: String, prop_name: String, old_prop_value: Variant, new_prop_value: Variant, label: String) -> void:
	var has_old = change_type != "added"
	var has_new = change_type != "removed"
	
	if label == null:
		label = snake_case_to_human_readable(prop_name)
	if has_old:
		var prop_editor = get_prop_editor(inspector_section.get_object(), prop_name + "_old", old_prop_value, removed_color, label)
		inspector_section.get_vbox().add_child(prop_editor)
	if has_new:
		var prop_editor = get_prop_editor(inspector_section.get_object(), prop_name + "_new", new_prop_value, added_color, label if !has_old else "")
		inspector_section.get_vbox().add_child(prop_editor)


func add_PropertyDiffResult(inspector_section: DiffInspectorSection, property_diff: Dictionary) -> void:
	var change_type = property_diff["change_type"]
	var prop_name = property_diff["name"]
	var prop_label = snake_case_to_human_readable(property_diff["name"])
	var prop_old = property_diff["old_value"]
	var prop_new = property_diff["new_value"]
	print("!!! adding property diff result for ", prop_name, " with type ", change_type)
	print("!!! prop_old: ", prop_old)
	print("!!! prop_new: ", prop_new)
	
	add_old_and_new(inspector_section, change_type, prop_name, prop_old, prop_new, prop_label)

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
	var inspector_section: DiffInspectorSection = DiffInspectorSection.new()
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


func get_prop_diffs_from_properties(properties: Dictionary, change_type: String) -> Dictionary:
	var prop_diffs: Dictionary = {}
	for prop in properties.keys():
		var prop_diff: Dictionary = {}
		prop_diff["name"] = prop
		prop_diff["change_type"] = change_type
		# TODO: fix this
		prop_diff["old_value"] = str_to_var(properties[prop])
		prop_diff["new_value"] = str_to_var(properties[prop])
		prop_diffs[prop] = prop_diff
	return prop_diffs

func add_NodeDiffResult(node_diff: Dictionary) -> void:
	var node_name: String = node_diff["node_path"] # remove the leading "./"
	var change_type: String = node_diff["change_type"]
	print("!!! adding node diff result for ", node_name, " with type ", change_type)
	
	var prop_diffs: Dictionary
	var inspector_section: DiffInspectorSection = DiffInspectorSection.new()
	var vbox = inspector_section.get_vbox()
	var fake_node = MissingResource.new()
	
	var node_type: String = ""
	if change_type == "added":
		inspector_section.setup(node_name, node_name, fake_node, added_color, true)
		# TODO: make rust code do this
		prop_diffs = get_prop_diffs_from_properties(node_diff["new_content"]["properties"], "added")
		node_type = node_diff["new_content"]["type"]
		print("adding node added box")
		vbox.add_child(get_node_added_box())
		added_nodes.append(fake_node)
	elif change_type == "removed":
		print("adding node deleted box")
		inspector_section.setup(node_name, node_name, fake_node, removed_color, true)
		# TODO: make rust code do this
		prop_diffs = get_prop_diffs_from_properties(node_diff["old_content"]["properties"], "removed")
		node_type = node_diff["old_content"]["type"]
		vbox.add_child(get_node_deleted_box())
		deleted_nodes.append(fake_node)
	else:
		inspector_section.setup(node_name, node_name, fake_node, modified_color, true)
		prop_diffs = node_diff["changed_props"]
		node_type = node_diff["new_content"]["type"]
		changed_nodes.append(fake_node)

	fake_node.original_class = node_type
	inspector_section.set_type(change_type)
	var i = 0
	# get the length of the prop_diffs dictionary
	var prop_diffs_length = prop_diffs.keys().size()
	print("prop_diffs_length: ", prop_diffs_length)
	for prop_name in prop_diffs.keys():
		if i > 0:
			var divider = HSeparator.new()
			vbox.add_child(divider)
		add_PropertyDiffResult(inspector_section, prop_diffs[prop_name])
		i += 1
	inspector_section.unfold()
	sections.append(inspector_section)
	main_vbox.add_child(inspector_section)



class DiffSet:
	var prop_name: String
	var change_type: String
	var old_prop_value: Variant
	var new_prop_value: Variant
	var label: String

func add_resource_diff(file_path: String, old_resource: Resource, new_resource: Resource) -> void:
	print("adding resource diff for ", file_path)
	if !is_instance_valid(old_resource) && !is_instance_valid(new_resource):
		return
	var prop_label = snake_case_to_human_readable(file_path)
	var has_old = is_instance_valid(old_resource)
	var has_new = is_instance_valid(new_resource)
	var change_type = "modified"
	if has_old && !has_new:
		change_type = "removed"
	elif !has_old && has_new:
		change_type = "added"
	elif has_old && has_new:
		change_type = "modified"
	var inspector_section: DiffInspectorSection = DiffInspectorSection.new()
	var vbox = inspector_section.get_vbox()
	var fake_node: MissingResource = MissingResource.new()
	fake_node.original_class = "Resource"
	inspector_section.setup(file_path, file_path, fake_node, modified_color, true)
	inspector_section.set_type(change_type)
	changed_resources.append(fake_node)
	add_old_and_new(inspector_section, change_type, "Resource", old_resource, new_resource, prop_label)
	main_vbox.add_child(inspector_section)
	sections.append(inspector_section)

func add_FileDiffResult(file_path: String, file_diff: Dictionary) -> void:
	var file_name = file_path
	var type = file_diff["diff_type"]
	print("!!! adding file diff result for ", file_name, " with type ", type)
	if type == "resource_changed":
		var res_old = file_diff["old_resource"]
		var res_new = file_diff["new_resource"]
		add_resource_diff(file_path, res_old, res_new)
	elif type == "scene_changed":
		var node_diffs: Array = file_diff["changed_nodes"]
		print("node_diff size: ", node_diffs.size())
		for node in node_diffs:
			var node_path: String = node["node_path"]
			# skip temporary nodes created by the instance
			if (node_path.contains("@")):
				continue
			add_NodeDiffResult(node)
			
# defs for these are in editor/diff_result.h
func add_diff(diff: Dictionary) -> void:
	print("ADDING DIFF!!!")
	diff_result = diff
	var file_diffs = diff.get("files")
	var size = file_diffs.size()
	print("Diff size: ", size)
	for file in file_diffs:
		print("Adding file diff result for ", file["path"])
		add_FileDiffResult(file["path"], file)

	
func reset() -> void:
	for section in sections:
		section.queue_free()
	sections.clear()
	categories.clear()
	for child in main_vbox.get_children():
		child.queue_free()
	changed_nodes.clear()
	added_nodes.clear()
	deleted_nodes.clear()
	changed_resources.clear()

	
	
func get_main_vbox() -> VBoxContainer:
	return main_vbox

func _init():
	pass

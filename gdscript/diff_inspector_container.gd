@tool
class_name DiffInspectorContainer
extends DiffInspector

@export var added_icon: Texture2D
@export var removed_icon: Texture2D
@export var modified_icon: Texture2D


func get_change_theme_color_name(change_type: String) -> String:
	if change_type == "added":
		return "prop_subsection_added"
	elif change_type == "removed":
		return "prop_subsection_removed"
	elif change_type == "modified":
		return "prop_subsection_modified"
	return "prop_subsection"


func set_inspector_change_color(name: String, color: Color) -> void:
	# get the theme override for the given name for the Editor type
	var theme: Theme = get_theme()
	theme.set_color(get_change_theme_color_name(name), "Editor", color)
	self.theme_changed.emit()


func get_color_for_change_type(change_type: String) -> Color:
	var theme: Theme = get_theme()
	return theme.get_color(get_change_theme_color_name(change_type), "Editor")


#3fa62e
@export var added_color: Color = Color("#3fa62e"):
	set(value):
		added_color = value
		set_inspector_change_color(("added"), value)

#a55454
@export var removed_color: Color = Color("#a55454"):
	set(value):
		removed_color = value
		set_inspector_change_color(("removed"), value)

#e2be99
@export var modified_color: Color = Color("#e2be99"):
	set(value):
		modified_color = value
		set_inspector_change_color(("modified"), value)

var diff_stylebox_tex = preload("./diff_stylebox_tex.png")
@onready var main_vbox: VBoxContainer = %DifferMainVBox
var diff_result: Dictionary

var categories: Array = []
var sections: Array = []
var node_map: Dictionary = {}
var changed_nodes: Array = []
var added_nodes: Array = []
var deleted_nodes: Array = []
# this is really just to keep a reference to the resources that have been changed;
var changed_resources: Array = []
var changed_files: Array = []
var waiting_callables: Array = []

func _ready() -> void:
	pass


# Called every frame. 'delta' is the elapsed time since the previous frame.
func _process(delta: float) -> void:
	var to_call = waiting_callables.duplicate()
	waiting_callables.clear()
	for callable in to_call:
		callable.call()

func _on_button_pressed() -> void:
	pass

# no type annotation for this because editor_property is ambiguously typed
func update_property_editor(editor_property) -> void:
	editor_property.set_read_only(true)
	editor_property.update_property()
	editor_property._update_editor_property_status()
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


func add_color_marker(change_type: String, panel_container: PanelContainer) -> void:
	var color_rect: ColorRect = ColorRect.new()
	color_rect.color = get_color_for_change_type(change_type)
	color_rect.custom_minimum_size = Vector2(10, 10)
	color_rect.layout_direction = 2 # horizontal
	color_rect.layout_mode = 2 # manual
	color_rect.size_flags_horizontal = 4 # expand
	var margin_container: MarginContainer = MarginContainer.new()
	margin_container.layout_mode = 2 # manual
	margin_container.add_theme_constant_override("margin_right", 20)
	margin_container.add_child(color_rect)
	panel_container.add_child(margin_container)
	var update_color_rect = func():
		if !is_instance_valid(color_rect):
			return
		color_rect.color = get_color_for_change_type(change_type)
		color_rect.theme_changed.emit()
		panel_container.queue_redraw()
	self.theme_changed.connect(update_color_rect)


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


func get_prop_editor(fake_object: MissingResource, prop_name: String, prop_value: Variant, change_type: String, prop_label: String) -> PanelContainer:
	# print("!!! getting prop editor for ", prop_name, " with value ", prop_value)
	fake_object.recording_properties = true
	fake_object.set(prop_name, prop_value)
	fake_object.recording_properties = false
	# print("!!! fake_object prop value: ", fake_object.get(prop_name))
	if prop_label == null:
		prop_label = snake_case_to_human_readable(prop_name)
	var editor_property: DiffInspectorProperty = DiffInspector.instance_property_diff(fake_object, prop_name, false)
	editor_property.set_object_and_property(fake_object, prop_name)
	update_property_editor(editor_property)
	var panel_container: PanelContainer = PanelContainer.new()
	add_label(prop_label, panel_container)
	add_color_marker(change_type, panel_container)
	panel_container.add_child(editor_property)
	changed_resources.append(fake_object)
	return panel_container

func add_old_and_new(inspector_section: DiffInspectorSection, change_type: String, prop_name: String, old_prop_value: Variant, new_prop_value: Variant, label: String) -> void:
	var has_old = change_type != "added"
	var has_new = change_type != "removed"

	if label == null:
		label = snake_case_to_human_readable(prop_name)
	if has_old:
		var prop_editor = get_prop_editor(inspector_section.get_object(), prop_name + "_old", old_prop_value, "removed", label)
		inspector_section.get_vbox().add_child(prop_editor)
	if has_new:
		var prop_editor = get_prop_editor(inspector_section.get_object(), prop_name + "_new", new_prop_value, "added", label if !has_old else "")
		inspector_section.get_vbox().add_child(prop_editor)


func add_PropertyDiffResult(inspector_section: DiffInspectorSection, property_diff: Dictionary) -> void:
	var change_type = property_diff["change_type"]
	var prop_name = property_diff["name"]
	var prop_label = snake_case_to_human_readable(property_diff["name"])
	var prop_old = property_diff["old_value"]
	var prop_new = property_diff["new_value"]
	# print("!!! adding property diff result for ", prop_name, " with type ", change_type)
	# print("!!! prop_old: ", prop_old)
	# print("!!! prop_new: ", prop_new)

	add_old_and_new(inspector_section, change_type, prop_name, prop_old, prop_new, prop_label)


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

func get_node_deleted_box(type: String) -> PanelContainer:
	return get_node_box(removed_icon, type + " Deleted")

func get_node_added_box(type: String) -> PanelContainer:
	return get_node_box(added_icon, type + " Added")


func get_prop_diffs_from_properties(properties: Dictionary, change_type: String) -> Dictionary:
	var prop_diffs: Dictionary = {}
	for prop in properties.keys():
		if prop.begins_with("metadata/patchwork_id"):
			continue
		var prop_diff: Dictionary = {}
		prop_diff["name"] = prop
		prop_diff["change_type"] = change_type
		# TODO: fix this
		prop_diff["old_value"] = str_to_var(properties[prop])
		prop_diff["new_value"] = str_to_var(properties[prop])
		prop_diffs[prop] = prop_diff
	return prop_diffs


func _on_node_box_clicked(section: String) -> void:
	print("!!! box clicked: ", section)
	if !section.begins_with("res://"):
		var file_path = node_map.get(section, "")
		if file_path == "":
			return
		var node_path = section
		if section.begins_with("./"):
			node_path = node_path.substr(2)

		if PatchworkEditor.is_changing_scene():
			waiting_callables.append(func():
				self._on_node_box_clicked(section)
			)
			return
		var node: Node = EditorInterface.get_edited_scene_root()
		if node.scene_file_path != file_path:
			EditorInterface.open_scene_from_path(file_path)
			waiting_callables.append(func():
				self._on_node_box_clicked(section)
			)
			return
		var node_to_select: Node = node.get_node(node_path)
		if is_instance_valid(node_to_select):
			EditorInterface.edit_node(node_to_select)
			EditorInterface.set_main_screen_editor("2D")

func _on_resource_box_clicked(section: String) -> void:
	var file_path = section
	var extension = file_path.get_extension().to_lower()
	if extension == "tscn" or extension == "scn":
		EditorInterface.open_scene_from_path(file_path)
		EditorInterface.set_main_screen_editor("2D")
	else:
		EditorInterface.get_inspector().edit_resource(ResourceLoader.load(file_path))

func _on_text_box_clicked(section: String) -> void:
	var file_path = section
	var extension = file_path.get_extension().to_lower()
	PatchworkEditor.open_script_file(file_path)
	EditorInterface.set_main_screen_editor("Script")



func add_NodeDiffResult(file_section: DiffInspectorSection, node_diff: Dictionary, parent_path: String) -> void:
	var node_name: String = node_diff["node_path"] # remove the leading "./"
	var node_label: String = node_name
	var change_type: String = node_diff["change_type"]
	# print("!!! adding node diff result for ", node_name, " with type ", change_type)

	var prop_diffs: Dictionary
	var inspector_section: DiffInspectorSection = DiffInspectorSection.new()
	var vbox = inspector_section.get_vbox()
	var fake_node = MissingResource.new()

	var node_type: String = ""
	var color: Color = added_color
	if change_type == "added":
		color = added_color
		node_label += " (Added)"
		# TODO: make rust code do this
		prop_diffs = get_prop_diffs_from_properties(node_diff["new_content"]["properties"], "added")
		# node_type = node_diff["new_content"]["type"]
		# print("adding node added box")
		added_nodes.append(fake_node)
	elif change_type == "removed":
		color = removed_color
		node_label += " (Deleted)"
		# print("adding node deleted box")
		prop_diffs = get_prop_diffs_from_properties(node_diff["old_content"]["properties"], "removed")
		# node_type = node_diff["old_content"]["type"]
		deleted_nodes.append(fake_node)
	else:
		color = modified_color
		node_label += " (Modified)"
		prop_diffs = node_diff["changed_props"]
		# node_type = node_diff["new_content"]["type"]
		changed_nodes.append(fake_node)
	inspector_section.setup(node_name, node_label, fake_node, color, true, 1, 2)
	inspector_section.set_type(change_type)
	# fake_node.original_class = node_type
	var i = 0
	# get the length of the prop_diffs dictionary
	var prop_diffs_length = prop_diffs.keys().size()
	# print("prop_diffs_length: ", prop_diffs_length)
	for prop_name in prop_diffs.keys():
		if i > 0:
			var divider = HSeparator.new()
			vbox.add_child(divider)
		add_PropertyDiffResult(inspector_section, prop_diffs[prop_name])
		i += 1
	inspector_section.unfold()
	inspector_section.connect("box_clicked", self._on_node_box_clicked)
	sections.append(inspector_section)
	node_map[node_name] = parent_path
	file_section.get_vbox().add_child(inspector_section)


# class DiffSet:
# 	var prop_name: String
# 	var change_type: String
# 	var old_prop_value: Variant
# 	var new_prop_value: Variant
# 	var label: String

func add_resource_diff(inspector_section: DiffInspectorSection, change_type: String, file_path: String, old_resource: Resource, new_resource: Resource) -> void:
	# print("adding resource diff for ", file_path)
	if !is_instance_valid(old_resource) && !is_instance_valid(new_resource):
		return
	var prop_label = snake_case_to_human_readable(file_path)
	var has_old = is_instance_valid(old_resource)
	var has_new = is_instance_valid(new_resource)
	var fake_node: MissingResource = MissingResource.new()
	fake_node.original_class = "Resource"
	changed_resources.append(fake_node)
	add_old_and_new(inspector_section, change_type, "Resource", old_resource, new_resource, prop_label)

func add_text_diff(inspector_section: DiffInspectorSection, unified_diff: Dictionary) -> void:
	# print("adding text diff")
	var text_diff = TextDiffer.get_text_diff(unified_diff, false)
	text_diff.custom_minimum_size = Vector2(100, 500)
	inspector_section.get_vbox().add_child(text_diff)


func add_FileDiffResult(file_path: String, file_diff: Dictionary) -> void:
	var file_name = file_path
	var label = file_name
	var type = file_diff.get("diff_type", "added_or_removed")
	var change_type = file_diff["change_type"]
	print("!!! adding file diff result for ", file_name, " with change_type ", change_type, " and type ", type)
	var color: Color
	if (change_type == "added"):
		color = added_color
		label += " (Added)"
		changed_files.append(file_path)
	elif (change_type == "removed"):
		color = removed_color
		label += " (Removed)"
		changed_files.append(file_path)
	elif (change_type == "modified"):
		color = modified_color
		label += " (Modified)"
		changed_files.append(file_path)
	var fake_node: MissingResource = MissingResource.new()
	changed_files.append(fake_node)
	var inspector_section: DiffInspectorSection = DiffInspectorSection.new()
	inspector_section.setup(file_path, label, fake_node, color, true)
	inspector_section.set_type(change_type)
	var vbox = inspector_section.get_vbox()
	if type == "added_or_removed":
		if change_type == "added":
			vbox.add_child(get_node_added_box("File"))
		elif change_type == "removed":
			vbox.add_child(get_node_deleted_box("File"))
	elif type == "resource_changed":
		var res_old = file_diff.get("old_resource", null)
		var res_new = file_diff.get("new_resource", null)
		add_resource_diff(inspector_section, change_type, file_path, res_old, res_new)
		inspector_section.connect("box_clicked", self._on_resource_box_clicked)
	elif type == "text_changed":
		var text_diff = file_diff["text_diff"]
		add_text_diff(inspector_section, text_diff)
		inspector_section.connect("box_clicked", self._on_text_box_clicked)
	elif type == "scene_changed":
		var node_diffs: Array = file_diff["changed_nodes"]
		node_diffs.sort_custom(func(a, b): return a["node_path"] < b["node_path"])
		inspector_section.connect("box_clicked", self._on_resource_box_clicked)

		# print("node_diff size: ", node_diffs.size())
		for node in node_diffs:
			var node_path: String = node["node_path"]
			# skip temporary nodes created by the instance
			if (node_path.contains("@")):
				continue
			add_NodeDiffResult(inspector_section, node, file_path)
	sections.append(inspector_section)
	main_vbox.add_child(inspector_section)

# defs for these are in editor/diff_result.h
func add_diff(diff: Dictionary) -> void:
	# print("ADDING DIFF!!!")
	diff_result = diff
	var size = diff_result.size()
	# print("Diff size: ", size)
	for file in diff_result.keys():
		if (file.to_lower().ends_with(".import")):
			continue
		# print("Adding file diff result for ", file)
		add_FileDiffResult(file, diff_result[file])


func reset() -> void:
	# disconnect from theme changed signal
	var all_signal_connections = self.theme_changed.get_connections()
	for connection in all_signal_connections:
		self.theme_changed.disconnect(connection.callable)
	for section in sections:
		section.queue_free()
	for child in main_vbox.get_children():
		child.queue_free()
	sections.clear()
	categories.clear()
	changed_nodes.clear()
	added_nodes.clear()
	deleted_nodes.clear()
	# changed_resources.clear()


func get_main_vbox() -> VBoxContainer:
	return main_vbox

func _init():
	pass

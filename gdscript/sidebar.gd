@tool
extends MarginContainer
# This is a Godot 4.x script file, written in GDScript 2.0. Connections are made using the identifier for the callable directly.
# Godot 3.x: something.connect("signal_name", self, "_on_signal_name")
# Godot 4.x: something.connect("signal_name", self._on_signal_name)

var godot_project: GodotProject

@onready var branch_picker: OptionButton = %BranchPicker
@onready var other_branch_picker: OptionButton = %OtherBranchPicker
@onready var menu_button: MenuButton = %MenuButton
@onready var history_list: ItemList = %HistoryList
@onready var user_button: Button = %UserButton
@onready var inspector: ScrollContainer = %PEEditorInspector
@onready var highlight_changes_checkbox: CheckBox = %HighlightChangesCheckbox
@onready var tab_container: TabContainer = %TabContainer

const TEMP_DIR = "user://tmp"

var branches = []
var other_branch_id
var merge_preview_active = false
var plugin: EditorPlugin
var config: PatchworkConfig
var current_inspector_object: FakeInspectorResource
var queued_calls = []

const CREATE_BRANCH_IDX = 1
const MERGE_BRANCH_IDX = 2

func init(plugin: EditorPlugin, godot_project: GodotProject, config: PatchworkConfig) -> void:
	print("Sidebar initialized!")
	self.godot_project = godot_project
	self.plugin = plugin
	self.config = config
	
func _update_ui_on_branches_changed(branches: Array):
	print("Branches changed, updating UI", branches)
	update_ui()

func _update_ui_on_files_saved():
	print("Files saved, updating UI")
	update_ui()

func _update_ui_on_files_changed():
	print("Files changed, updating UI")
	update_ui()

func _update_ui_on_branch_checked_out(branch):
	print("Branch checked out, updating UI")
	update_ui()

func _on_resource_saved(path):
	print("Resource saved: %s" % [path])

func _on_scene_saved(path):
	print("Scene saved: %s" % [path])

# TODO: It seems that Sidebar is being instantiated by the editor before the plugin does?
func _ready() -> void:
	print("sidebar ready")

	update_ui()

	# @Paul: I think somewhere besides the plugin sidebar gets instantiated. Is this something godot does?
	# to paper over this we check if plugin and godot_project are set

	if plugin:
		plugin.connect("resource_saved", self._on_resource_saved)
		plugin.connect("scene_saved", self._on_scene_saved)
	
	if godot_project:
		godot_project.connect("branches_changed", self._update_ui_on_branches_changed);
		godot_project.connect("saved_changes", self._update_ui_on_files_changed);
		godot_project.connect("files_changed", self._update_ui_on_files_changed);
		godot_project.connect("checked_out_branch", self._update_ui_on_branch_checked_out);
		
	var popup = menu_button.get_popup()
	popup.id_pressed.connect(_on_menu_button_id_pressed)
	user_button.pressed.connect(_on_user_button_pressed)
	branch_picker.item_selected.connect(_on_branch_picker_item_selected)
	other_branch_picker.item_selected.connect(_on_other_branch_picker_item_selected)
	highlight_changes_checkbox.toggled.connect(_on_highlight_changes_checkbox_toggled)
	tab_container.tab_changed.connect(_on_tab_container_tab_changed)
func _on_tab_container_tab_changed(tab_idx: int) -> void:
	var tab_name = tab_container.get_tab_title(tab_idx)

	if tab_name == "Merge Preview 🧪":
		merge_preview_active = true
		checkout_branch(godot_project.get_checked_out_branch().id, [other_branch_id])
	else:
		print("resetting merge preview")
		merge_preview_active = false
		checkout_branch(godot_project.get_checked_out_branch().id, [])

func _on_menu_button_id_pressed(id: int) -> void:
	match id:
		CREATE_BRANCH_IDX:
			create_new_branch()

		MERGE_BRANCH_IDX:
			merge_current_branch()

func _on_user_button_pressed():
	var dialog = ConfirmationDialog.new()
	dialog.title = "Set User Name"
	
	var line_edit = LineEdit.new()
	line_edit.placeholder_text = "User name"
	line_edit.text = config.get_user_value("user_name", "")
	dialog.add_child(line_edit)
	
	# Position line edit in dialog
	line_edit.position = Vector2(8, 8)
	line_edit.size = Vector2(200, 30)
	
	# Make dialog big enough for line edit
	dialog.size = Vector2(220, 100)
	
	dialog.get_ok_button().text = "Save"
	dialog.canceled.connect(func(): dialog.queue_free())
	
	dialog.confirmed.connect(func():
		if line_edit.text.strip_edges() != "":
			var new_user_name = line_edit.text.strip_edges()

			print(new_user_name)
			config.set_user_value("user_name", new_user_name)
			godot_project.set_user_name(new_user_name)

		update_ui()
		dialog.queue_free()
	)
	
	add_child(dialog)
	dialog.popup_centered()

func _on_branch_picker_item_selected(index: int) -> void:
	var selected_branch = branches[index]

	# reset selection in branch picker in case checkout_branch fails
	# once branch is actually checked out, the branch picker will update
	update_ui()

	if selected_branch.is_not_loaded:
		# Show warning dialog that branch is not synced correctly
		var dialog = AcceptDialog.new()
		dialog.title = "Branch Not Available"
		dialog.dialog_text = "Can't checkout branch because it is not synced yet"
		dialog.get_ok_button().text = "OK"
		dialog.canceled.connect(func(): dialog.queue_free())
		dialog.confirmed.connect(func(): dialog.queue_free())
		
		add_child(dialog)
		dialog.popup_centered()
		
		# Return early to prevent checkout attempt
		return


	print("picked in branch picker: ", index, " ", selected_branch)


	if merge_preview_active:
		checkout_branch(selected_branch.id, [other_branch_id])
	else:
		checkout_branch(selected_branch.id, [])


func _on_other_branch_picker_item_selected(index: int) -> void:
	var selected_branch = branches[index]
	other_branch_id = selected_branch.id

	print("other branch id: ", other_branch_id)

	checkout_branch(godot_project.get_checked_out_branch().id, [other_branch_id])


func _on_highlight_changes_checkbox_toggled(pressed: bool) -> void:
	update_ui()

static var void_func = func(): return
static func popup_box(parent_window: Node, dialog: AcceptDialog, message: String, box_title: String, confirm_func: Callable = void_func, cancel_func: Callable = void_func):
	if (dialog == null):
		dialog = AcceptDialog.new()
	if (dialog.get_parent() != parent_window):
		if (dialog.get_parent() == null):
			parent_window.add_child(dialog)
		else:
			dialog.reparent(parent_window)
	dialog.reset_size()
	dialog.set_text(message)
	dialog.set_title(box_title)
	var _confirm_func: Callable
	var _cancel_func: Callable
	var arr = dialog.get_signal_connection_list("confirmed")
	for dict in arr:
		dialog.disconnect("confirmed", dict.callable)
	arr = dialog.get_signal_connection_list("canceled")
	for dict in arr:
		dialog.disconnect("canceled", dict.callable)
	dialog.connect("confirmed", confirm_func)
	dialog.connect("canceled", cancel_func)
	dialog.popup_centered()

var current_cvs_action = []

# The reason we are queueing calls is because we need to wait for the editor to finish performing certain actions before we can manipulate gui elements
# We were getting crashes and weird behavior when trying to do everything synchronously
# Basically, if we need to do something that would induce the editor to perform actions, we need to queue it
func add_call_to_queue(call: Callable):
	queued_calls.append(call)

func _before_cvs_action_after_save(cvs_action: String, callback: Callable, save: bool):
	PatchworkEditor.progress_add_task(cvs_action, cvs_action, 10, false)
	current_cvs_action.append(cvs_action)
	if save:
		plugin.sync_godot_to_patchwork()
		plugin.file_system.connect_to_file_system()
	print("*** All scenes saved!")
	add_call_to_queue(callback)


func _before_cvs_action(cvs_action: String, callback: Callable, save: bool):
	if not save:
		add_call_to_queue(self._before_cvs_action_after_save.bind(cvs_action, callback, save))
		return
	print("*** Saving all scenes before CVS action %s" % [cvs_action])
	plugin.file_system.disconnect_from_file_system()
	EditorInterface.save_all_scenes();
	add_call_to_queue(self._before_cvs_action_after_save.bind(cvs_action, callback, save))


func _after_cvs_action():
	if not current_cvs_action.is_empty():
		for i in range(current_cvs_action.size()):
			pass
			PatchworkEditor.progress_end_task(current_cvs_action[i])
		current_cvs_action = []


func ensure_user_has_no_unsaved_files(message: String, callback: Callable):
	# todo: add back auto save
	if PatchworkEditor.unsaved_files_open():
		var dialog = AcceptDialog.new()
		dialog.title = "Unsaved Files"
		dialog.dialog_text = message
		dialog.get_ok_button().text = "OK"
		
		dialog.confirmed.connect(func():
			dialog.queue_free()
		)
		
		add_child(dialog)
		dialog.popup_centered()
		return

	else:
		callback.call()


func checkout_branch(branch_id: String, secondary_branch_ids: Array) -> void:
	ensure_user_has_no_unsaved_files("You have unsaved files open. You need to save them before checking out another branch.", func():
		_before_cvs_action("checking out", func():
			godot_project.checkout_branch(branch_id, secondary_branch_ids)
			add_call_to_queue(self._after_cvs_action)
		, false)
	)

func create_new_branch() -> void:
	var message = "creating a new branch"

	ensure_user_has_no_unsaved_files("You have unsaved files open. You need to save them before creating a new branch.", func():
		var dialog = ConfirmationDialog.new()
		dialog.title = "Create New Branch"
		
		var branch_name_input = LineEdit.new()
		branch_name_input.placeholder_text = "Branch name"
		dialog.add_child(branch_name_input)
		
		# Position line edit in dialog
		branch_name_input.position = Vector2(8, 8)
		branch_name_input.size = Vector2(200, 30)
		
		# Make dialog big enough for line edit
		dialog.size = Vector2(220, 100)
	

		# Disable create button if title is empty
		dialog.get_ok_button().disabled = true
		branch_name_input.text_changed.connect(func(new_text: String):
			if new_text.strip_edges() == "":
				dialog.get_ok_button().disabled = true
			else:
				dialog.get_ok_button().disabled = false
		)

		dialog.get_ok_button().text = "Create"
		
		dialog.canceled.connect(func(): dialog.queue_free())

		dialog.confirmed.connect(func():
			var new_branch_name = branch_name_input.text.strip_edges()
			dialog.queue_free()

			_before_cvs_action("creating a new branch", func():
				godot_project.create_branch(new_branch_name)
				add_call_to_queue(self._after_cvs_action)
			, false)
		)

		add_child(dialog)

		dialog.popup_centered()

		# focus on the branch name input
		branch_name_input.grab_focus()
	)

func merge_current_branch():
	var checked_out_branch = godot_project.get_checked_out_branch()

	if checked_out_branch.is_main:
		popup_box(self, $ErrorDialog, "Can't merge the main branch!", "Error")
		return
	ensure_user_has_no_unsaved_files("You have unsaved files open. You need to save them before merging.", func():
		popup_box(self, $ConfirmationDialog, "Are you sure you want to merge \"%s\" into main ?" % [checked_out_branch.name], "Merge Branch", func():
			_before_cvs_action("merging", func():
				godot_project.merge_branch(checked_out_branch.id)
				add_call_to_queue(self._after_cvs_action)
			, false)
		)
	)


func update_ui() -> void:
	if not godot_project:
		return

	var checked_out_branch = godot_project.get_checked_out_branch()

	self.branches = godot_project.get_branches()

	# highlight changes

	var edited_root = EditorInterface.get_edited_scene_root()

	if edited_root:
		if highlight_changes_checkbox.is_pressed() && !checked_out_branch.is_main:
				var edited_scene_file_path = edited_root.scene_file_path
				var node_changes = godot_project.get_node_changes(edited_scene_file_path)

				print("adding highlight")
				HighlightChangesLayer.highlight_changes(edited_root, node_changes)
		else:
			print("removing highlight")
			HighlightChangesLayer.remove_highlight(edited_root)


	# update branch pickers

	branch_picker.clear()
	other_branch_picker.clear()
	other_branch_picker.add_item("---")
	
	print("UI: checked out branch: ", checked_out_branch)

	for i in range(branches.size()):
		var branch = branches[i]
		var label = branch.name
		var is_checked_out = checked_out_branch && branch.id == checked_out_branch.id

		if branch.is_main:
			label = label + " 👑"

		branch_picker.add_item(label, i)
		branch_picker.set_item_metadata(i, branch.id)

		if !is_checked_out:
			other_branch_picker.add_item(label, i)
			other_branch_picker.set_item_metadata(i, branch.id)

			if branch.id == other_branch_id:
				other_branch_picker.select(i)


		# this should not happen, but right now the sync is not working correctly so we need to surface this in the interface
		if branch.is_not_loaded:
			branch_picker.set_item_icon(i, load("res://addons/patchwork/icons/warning.svg"))

		if is_checked_out:
			branch_picker.select(i)

	# update history
	
	var history = godot_project.get_changes()
	history_list.clear()

	print("changes:", history.size())

	for change in history:
		var change_hash = change.hash.substr(0, 7)
		var change_author = change.user_name
		var change_timestamp = human_readable_timestamp(change.timestamp)


		history_list.add_item(change_hash + " - " + change_author + " - " + change_timestamp)

	# update context menu

	var menu_popup = menu_button.get_popup()
	
	menu_popup.clear()

	menu_popup.add_item("Create new branch", CREATE_BRANCH_IDX) # Create new branch menu item
	menu_popup.add_item("Merge branch", MERGE_BRANCH_IDX)

	# update user name

	var user_name = config.get_user_value("user_name", "")

	user_button.text = user_name

	# update properties diff
	update_properties_diff()

func human_readable_timestamp(timestamp: int) -> String:
	var now = Time.get_unix_time_from_system() * 1000 # Convert to ms
	var diff = (now - timestamp) / 1000 # Convert diff to seconds
	
	if diff < 60:
		return str(int(diff)) + " seconds ago"
	elif diff < 3600:
		return str(int(diff / 60)) + " minutes ago"
	elif diff < 86400:
		return str(int(diff / 3600)) + " hours ago"
	elif diff < 604800:
		return str(int(diff / 86400)) + " days ago"
	elif diff < 2592000:
		return str(int(diff / 604800)) + " weeks ago"
	elif diff < 31536000:
		return str(int(diff / 2592000)) + " months ago"
	else:
		return str(int(diff / 31536000)) + " years ago"


var prev_heads_before
var prev_heads_after

func update_properties_diff() -> void:
	var checked_out_branch = godot_project.get_checked_out_branch()

	inspector.visible = !checked_out_branch.is_main;

	print("checked_out_branch: ", checked_out_branch)

	if !inspector.visible:
		return

	var change: Array[Dictionary] = godot_project.get_changes()
	if (change.size() < 2):
		return
	

	var heads_before = checked_out_branch.forked_at
	var heads_after = godot_project.get_heads()


	if (prev_heads_before == heads_before && prev_heads_after == heads_after):
		return

	prev_heads_before = heads_before
	prev_heads_after = heads_after

	show_diff(heads_before, heads_after)

	_cleanup_inspector(inspector)
	

# hides the extra nodes in the inspector that we don't want to show
# this happens because we just use the existing inspector to show the diff
func _cleanup_inspector(node: Node) -> void:
	var tooltip_text = node.get("tooltip_text")

	if (tooltip_text == "." || tooltip_text == "Resource"):
		node.visible = false
		return


	for child in node.get_children():
		_cleanup_inspector(child)

func show_diff(heads_before, heads_after):
	# TODO: handle dependencies of these files
	var diff_dict = godot_project.get_changed_file_content_between(PackedStringArray(heads_before), PackedStringArray(heads_after))
	var files_arr = diff_dict["files"]
	if files_arr.size() == 0:
		#print("No changes between %s and %s" % [hash1, hash2])
		return
	# print("Changes between %s and %s:" % [hash1, hash2])
	var new_dict = {}
	var new_files = []
	for file: Dictionary in files_arr:
		var path = file["path"]
		var change = file["change"]
		var old_content = file["old_content"]
		var new_content = file["new_content"]
		# for all the files in the dict, save as tmp files

		print("File: %s" % path)
		print("Change: %s" % change)
		var old_path = TEMP_DIR.path_join(path.trim_prefix("res://")).get_basename() + "_old." + path.get_extension()
		var new_path = TEMP_DIR.path_join(path.trim_prefix("res://")).get_basename() + "_new." + path.get_extension()
		# print("Old path: %s" % old_path)
		#	print("New path: %s" % new_path)
		if change == "added":
			old_path = null
		if change == "deleted":
			new_path = null
		if old_path:
			plugin.file_system.save_file(old_path, old_content)
		if new_path:
			plugin.file_system.save_file(new_path, new_content)
		new_files.append({
			"path": path,
			"change": change,
			"old_content": old_path,
			"new_content": new_path
		})
	new_dict["files"] = new_files
	var display_diff = PatchworkEditor.get_diff(new_dict)
	current_inspector_object = FakeInspectorResource.new()
	current_inspector_object.recording_properties = true
	for file in display_diff.keys():
		current_inspector_object.add_file_diff(file, display_diff[file])
	inspector.edit_object = current_inspector_object

func _process(delta: float) -> void:
	if queued_calls.size() == 0:
		return
	var calls = []
	calls.append_array(queued_calls)
	queued_calls.clear()
	for call in calls:
		if call.is_valid():
			call.call()
		else:
			print("Invalid call: ", call)

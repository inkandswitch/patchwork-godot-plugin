@tool
extends MarginContainer
# This is a Godot 4.x script file, written in GDScript 2.0. Connections are made using the identifier for the callable directly.
# Godot 3.x: something.connect("signal_name", self, "_on_signal_name")
# Godot 4.x: something.connect("signal_name", self._on_signal_name)

const diff_inspector_script = preload("res://addons/patchwork/gdscript/diff_inspector_container.gd")
@onready var branch_picker: OptionButton = %BranchPicker
@onready var history_list: ItemList = %HistoryList
@onready var user_button: Button = %UserButton
@onready var highlight_changes_checkbox: CheckBox = %HighlightChangesCheckbox
@onready var highlight_changes_checkbox_mp: CheckBox = %HighlightChangesCheckboxMP
@onready var inspector: DiffInspectorContainer = %BigDiffer
@onready var merge_preview_modal: Control = %MergePreviewModal
@onready var cancel_merge_button: Button = %CancelMergeButton
@onready var confirm_merge_button: Button = %ConfirmMergeButton
@onready var merge_preview_title: Label = %MergePreviewTitle
@onready var merge_preview_source_label: Label = %MergePreviewSourceLabel
@onready var merge_preview_target_label: Label = %MergePreviewTargetLabel
@onready var merge_preview_diff_container: MarginContainer = %MergePreviewDiffContainer
@onready var main_diff_container: MarginContainer = %MainDiffContainer
@onready var merge_preview_message_label: Label = %MergePreviewMessageLabel
@onready var merge_preview_message_icon: TextureRect = %MergePreviewMessageIcon
@onready var sync_status_icon: TextureButton = %SyncStatusIcon
@onready var fork_button: Button = %ForkButton
@onready var merge_button: Button = %MergeButton

@onready var history_section_header: Button = %HistorySectionHeader
@onready var history_section_body: Control = %HistorySectionBody
@onready var diff_section_header: Button = %DiffSectionHeader
@onready var diff_section_body: Control = %DiffSectionBody
@onready var branch_picker_cover: Button = %BranchPickerCover

const TEMP_DIR = "user://tmp"

var DEBUG_MODE = false

var plugin: EditorPlugin

var task_modal: TaskModal = TaskModal.new()

var highlight_changes = false

const CREATE_BRANCH_IDX = 1
const MERGE_BRANCH_IDX = 2

func init() -> void:
	print("Sidebar initialized!")


func _update_ui_on_branches_changed(_branches: Array):
	update_ui()

func _update_ui_on_files_saved():
	update_ui()

func _update_ui_on_files_changed():
	update_ui()

func _update_ui_on_branch_checked_out(_branch):
	update_ui()

# TODO: It seems that Sidebar is being instantiated by the editor before the plugin does?
func _ready() -> void:
	# need to add task_modal as a child to the plugin otherwise process won't be called
	add_child(task_modal)

	update_sync_status()
	update_ui()

	# get the class name of the inspector
	var inspector_class = inspector.get_class()
	print("Inspector class: ", inspector_class)
	inspector.get_script()
	# @Paul: I think somewhere besides the plugin sidebar gets instantiated. Is this something godot does?
	# to paper over this we check if plugin and godot_project are set

	if GodotProject.get_singleton():
		GodotProject.connect("branches_changed", self._update_ui_on_branches_changed);
		GodotProject.connect("saved_changes", self._update_ui_on_files_changed);
		GodotProject.connect("files_changed", self._update_ui_on_files_changed);
		GodotProject.connect("checked_out_branch", self._update_ui_on_branch_checked_out);
		GodotProject.connect("sync_server_connection_info_changed", _on_sync_server_connection_info_changed)


	merge_button.pressed.connect(create_merge_preview_branch)
	fork_button.pressed.connect(create_new_branch)

	user_button.pressed.connect(_on_user_button_pressed)
	branch_picker.item_selected.connect(_on_branch_picker_item_selected)

	highlight_changes_checkbox.toggled.connect(_on_highlight_changes_checkbox_toggled)
	highlight_changes_checkbox_mp.toggled.connect(_on_highlight_changes_checkbox_toggled)
	cancel_merge_button.pressed.connect(cancel_merge_preview)
	confirm_merge_button.pressed.connect(confirm_merge_preview)

	history_section_header.pressed.connect(func(): toggle_section(history_section_header, history_section_body))
	diff_section_header.pressed.connect(func(): toggle_section(diff_section_header, diff_section_body))


func _on_sync_server_connection_info_changed(_peer_connection_info: Dictionary) -> void:
	update_sync_status()

func update_sync_status() -> void:
	var checked_out_branch = GodotProject.get_checked_out_branch()
	if !checked_out_branch:
		return

	var peer_connection_info = GodotProject.get_sync_server_connection_info()
	print("update sync status", peer_connection_info)
	if !peer_connection_info:
		printerr("no peer connection info")
		return

	# check if doc_sync_states has the checked_out_branch.id
	if !peer_connection_info.doc_sync_states.has(checked_out_branch.id):
		return

	var sync_status = peer_connection_info.doc_sync_states[checked_out_branch.id];

	if sync_status.last_acked_heads == checked_out_branch.heads:
		sync_status_icon.texture = load("res://addons/patchwork/icons/circle-check.svg")
		sync_status_icon.tooltip_text = "Fully synced"
	else:
		sync_status_icon.texture = load("res://addons/patchwork/icons/circle_sync.svg")
		sync_status_icon.tooltip_text = "Syncing..."


func _on_user_button_pressed():
	var dialog = ConfirmationDialog.new()
	dialog.title = "Set User Name"

	var line_edit = LineEdit.new()
	line_edit.placeholder_text = "User name"
	line_edit.text = PatchworkConfig.get_user_value("user_name", "")
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
			PatchworkConfig.set_user_value("user_name", new_user_name)
			GodotProject.set_user_name(new_user_name)

		update_ui()
		dialog.queue_free()
	)

	add_child(dialog)
	dialog.popup_centered()

func _on_branch_picker_item_selected(_index: int) -> void:
	var selected_branch = branch_picker.get_item_metadata(_index)

	# reset selection in branch picker in case checkout_branch fails
	# once branch is actually checked out, the branch picker will update
	update_ui()

	if "is_not_loaded" in selected_branch && selected_branch.is_not_loaded:
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

	checkout_branch(selected_branch.id)


func _on_highlight_changes_checkbox_toggled(pressed: bool) -> void:
	highlight_changes = pressed
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

func checkout_branch(branch_id: String) -> void:
	var branch = GodotProject.get_branch_by_id(branch_id)
	if (!branch):
		popup_box(self, $ErrorDialog, "Branch not found", "Error")
		return

	ensure_user_has_no_unsaved_files("You have unsaved files open. You need to save them before checking out another branch.", func():
		task_modal.do_task(
			"Checking out branch \"%s\"" % [branch.name],
			func():
				GodotProject.checkout_branch(branch_id)

				if GodotProject.get_checked_out_branch().id != branch_id:
					await GodotProject.checked_out_branch
		)
	)

func create_new_branch() -> void:
	ensure_user_has_no_unsaved_files("You have unsaved files open. You need to save them before creating a new branch.", func():
		var dialog = ConfirmationDialog.new()
		dialog.title = "Create New Branch"

		var branch_name_input = LineEdit.new()
		branch_name_input.placeholder_text = "Branch name"

		var user_name = PatchworkConfig.get_user_value("user_name", "")
		if !user_name:
			user_name = "Anonymous"

		branch_name_input.text = user_name + "'s remix"
		dialog.add_child(branch_name_input)

		# Position line edit in dialog
		branch_name_input.position = Vector2(8, 8)
		branch_name_input.size = Vector2(200, 30)

		# Make dialog big enough for line edit
		dialog.size = Vector2(220, 100)

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

			task_modal.do_task("Creating new branch \"%s\"" % [new_branch_name], func():
				GodotProject.create_branch(new_branch_name)

				await GodotProject.checked_out_branch
			)
		)

		add_child(dialog)

		dialog.popup_centered()

		# focus on the branch name input
		branch_name_input.grab_focus()
	)

func move_inspector_to_merge_preview() -> void:
	if inspector and main_diff_container and merge_preview_diff_container and inspector.get_parent() != merge_preview_diff_container:
		inspector.reparent(merge_preview_diff_container)
		inspector.visible = true

func move_inspector_to_main() -> void:
	if inspector and main_diff_container and merge_preview_diff_container and inspector.get_parent() != main_diff_container:
		inspector.reparent(main_diff_container)
		inspector.visible = true

func create_merge_preview_branch():
	var checked_out_branch = GodotProject.get_checked_out_branch()
	if checked_out_branch.is_main:
		popup_box(self, $ErrorDialog, "Can't merge the main branch!", "Error")
		return

	var forked_from_branch = GodotProject.get_branch_by_id(checked_out_branch.forked_from)

	var source_branch_doc_id = checked_out_branch.id
	var target_branch_doc_id = forked_from_branch.id

	task_modal.do_task("Creating merge preview", func():
		GodotProject.create_merge_preview_branch(source_branch_doc_id, target_branch_doc_id)

		await GodotProject.checked_out_branch
	)

func cancel_merge_preview():
	task_modal.do_task("Cancel merge preview", func():
		var checked_out_branch = GodotProject.get_checked_out_branch()

		GodotProject.delete_branch(checked_out_branch.id)
		GodotProject.checkout_branch(checked_out_branch.forked_from)
	)


func confirm_merge_preview():
	var checked_out_branch = GodotProject.get_checked_out_branch()

	var source_branch_doc_id = checked_out_branch.id
	var target_branch_doc_id = checked_out_branch.merge_into

	var original_source_branch = GodotProject.get_branch_by_id(checked_out_branch.forked_from)
	var target_branch = GodotProject.get_branch_by_id(checked_out_branch.merge_into)

	ensure_user_has_no_unsaved_files("You have unsaved files open. You need to save them before merging.", func():
		popup_box(self, $ConfirmationDialog, "Are you sure you want to merge \"%s\" into \"%s\" ?" % [original_source_branch.name, target_branch.name], "Merge Branch", func():
			task_modal.do_task("Merging \"%s\" into \"%s\"" % [original_source_branch.name, target_branch.name], func():
				GodotProject.merge_branch(source_branch_doc_id, target_branch_doc_id)
			)
		)
	)

func toggle_section(section_header: Button, section_body: Control):
	if section_body.visible:
		section_header.icon = load("res://addons/patchwork/icons/collapsable-closed.svg")
		section_body.visible = false
	else:
		section_header.icon = load("res://addons/patchwork/icons/collapsable-open.svg")
		section_body.visible = true

func unfold_section(section_header: Button, section_body: Control):
	section_header.icon = load("res://addons/patchwork/icons/collapsable-open.svg")
	section_body.visible = true

func fold_section(section_header: Button, section_body: Control):
	section_header.icon = load("res://addons/patchwork/icons/collapsable-closed.svg")
	section_body.visible = false

func update_ui() -> void:
	var checked_out_branch = GodotProject.get_checked_out_branch()

	# update branch pickers

	update_branch_picker()

	# update history

	var history = GodotProject.get_changes()
	history_list.clear()

	for i in range(history.size() - 1, -1, -1):
		var change = history[i]

		if change.branch_id != checked_out_branch.id:
			continue

		var change_author = change.username
		var change_timestamp = human_readable_timestamp(change.timestamp)

		var prefix = ""
		
		if DEBUG_MODE:
			prefix = change.hash.substr(0, 8) + " - "

		if "merge_metadata" in change:
			var merged_branch = GodotProject.get_branch_by_id(change.merge_metadata.merged_branch_id)
			var merged_branch_name = merged_branch.name
			history_list.add_item(prefix + "↪️ merged \"" + merged_branch_name + "\" branch - by " + change_author + " " + change_timestamp)

		else:
			history_list.add_item(prefix + "some changes -  by " + change_author + " " + change_timestamp + "")
	
	# update action buttons

	if checked_out_branch && checked_out_branch.is_main:
		merge_button.disabled = true
		merge_button.tooltip_text = "Can't merge main because it's not a remix of another branch"
	else:
		merge_button.disabled = false
		merge_button.tooltip_text = ""
	

	# update user name

	var user_name = PatchworkConfig.get_user_value("user_name", "")

	user_button.text = user_name

	# update merge preview

	if !checked_out_branch:
		return

	merge_preview_modal.visible = checked_out_branch.is_merge_preview

	if checked_out_branch.is_merge_preview:
		move_inspector_to_merge_preview()

		var source_branch = GodotProject.get_branch_by_id(checked_out_branch.forked_from)
		var target_branch = GodotProject.get_branch_by_id(checked_out_branch.merge_into)

		if source_branch && target_branch:
			merge_preview_source_label.text = source_branch.name
			merge_preview_target_label.text = target_branch.name
			merge_preview_title.text = "Preview of \"" + target_branch.name + "\""

			if checked_out_branch.merge_at != checked_out_branch.forked_at:
				merge_preview_message_label.text = "Becarfultoreviewyourchanges and makesurethegame is stillworkingcorrectlybeforemerging.\nThere have been changes to \"" + target_branch.name + "\" since \"" + source_branch.name + "\" was created."
				merge_preview_message_icon.texture = load("res://addons/patchwork/icons/warning-circle.svg")
			else:
				merge_preview_message_label.text = "This branch is safe to merge.\nThere have been no changes to \"" + target_branch.name + "\" since \"" + checked_out_branch.name + "\" was created."
				merge_preview_message_icon.texture = load("res://addons/patchwork/icons/checkmark-circle.svg")

	else:
		move_inspector_to_main()

	# DIFF

	# show no diff for main branch
	if checked_out_branch.is_main:
		update_highlight_changes({}, checked_out_branch)
		inspector.visible = false

	else:
		var heads_before
		var heads_after

		if checked_out_branch.is_merge_preview:
			heads_before = checked_out_branch.merge_at
			heads_after = checked_out_branch.heads
		else:
			heads_before = checked_out_branch.forked_at
			heads_after = checked_out_branch.heads


		print("heads_before: ", heads_before)
		print("heads_after: ", heads_after)

		var diff = update_properties_diff(heads_before, heads_after)

		inspector.visible = true


		update_highlight_changes(diff, checked_out_branch)

func update_branch_picker() -> void:
	branch_picker.clear()

	var checked_out_branch = GodotProject.get_checked_out_branch()

	if !checked_out_branch:
		return

	var main_branch = GodotProject.get_main_branch()

	if !main_branch:
		return

	var all_branches = GodotProject.get_branches()

	branch_picker_cover.text = checked_out_branch.name
	add_branch_with_forks(main_branch, all_branches, checked_out_branch.id)

func add_branch_with_forks(branch: Dictionary, all_branches: Array, selected_branch_id: String, indentation: String = "", is_last: bool = false) -> void:
	var label
	if branch.is_main:
		label = branch.name
	else:
		var connection
		if is_last:
			connection = "└─ "
		else:
			connection = "├─ "

		label = indentation + connection + branch.name


	var branch_index = branch_picker.get_item_count()
	branch_picker.add_item(label, branch_index)

	# this should not happen, but right now the sync is not working correctly so we need to surface this in the interface
	if branch.is_not_loaded:
		branch_picker.set_item_icon(branch_index, load("res://addons/patchwork/icons/warning.svg"))

	branch_picker.set_item_metadata(branch_index, branch)

	if branch.id == selected_branch_id:
		branch_picker.select(branch_index)

	var new_indentation = ""
	if branch.is_main:
		new_indentation = ""
	else:
		if is_last:
			new_indentation = indentation + "    "
		else:
			new_indentation = indentation + "│   "


	var forked_off_branches = []
	for other_branch in all_branches:
		if !("forked_from" in other_branch):
			continue

		if other_branch.forked_from != branch.id:
			continue

		forked_off_branches.append(other_branch)

	for child_index in range(forked_off_branches.size()):
		var forked_off_branch = forked_off_branches[child_index]
		var is_last_child = child_index == forked_off_branches.size() - 1
		add_branch_with_forks(forked_off_branch, all_branches, selected_branch_id, new_indentation, is_last_child)

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

func update_highlight_changes(diff: Dictionary, checked_out_branch) -> void:
	var edited_root = EditorInterface.get_edited_scene_root()

	# reflect highlight changes checkbox state
	highlight_changes_checkbox_mp.button_pressed = highlight_changes
	highlight_changes_checkbox.button_pressed = highlight_changes

	if edited_root:
		if highlight_changes && !checked_out_branch.is_main:
				var path = edited_root.scene_file_path
				var scene_changes = diff.get(path)
				if scene_changes:
					HighlightChangesLayer.highlight_changes(edited_root, scene_changes)
		else:
			print("removing highlight")
			HighlightChangesLayer.remove_highlight(edited_root)


var prev_heads_before
var prev_heads_after
var last_diff: Dictionary = {}
func update_properties_diff(heads_before, heads_after) -> Dictionary:
	var checked_out_branch = GodotProject.get_checked_out_branch()

	if (!inspector):
		return last_diff
	if (!checked_out_branch):
		return last_diff

	var change: Array[Dictionary] = GodotProject.get_changes()
	if (change.size() < 2):
		return last_diff

	if (prev_heads_before == heads_before && prev_heads_after == heads_after):
		return last_diff

	prev_heads_before = heads_before
	prev_heads_after = heads_after
	last_diff = show_diff(heads_before, heads_after)
	return last_diff


func show_diff(heads_before, heads_after):
	# TODO: handle dependencies of these files
	# print("heads_before: ", heads_before)
	# print("heads_after: ", heads_after)
	var diff = GodotProject.get_all_changes_between(PackedStringArray(heads_before), PackedStringArray(heads_after))
	inspector.reset()
	inspector.add_diff(diff)
	print("Length: ", diff.size())
	return diff

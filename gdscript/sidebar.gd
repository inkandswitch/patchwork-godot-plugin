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

const DIFF_SECTION_HEADER_TEXT_FORMAT = "Changes: Showing diff between %s and %s"

const TEMP_DIR = "user://tmp"

var DEBUG_MODE = false

var plugin: EditorPlugin

var task_modal: TaskModal = TaskModal.new()

var highlight_changes = false

var waiting_callables: Array = []

var deterred_highlight_update = null


const CREATE_BRANCH_IDX = 1
const MERGE_BRANCH_IDX = 2

signal reload_ui();


func _update_ui_on_branches_changed(_branches: Array):
	print("update_ui_on_branches_changed")
	update_ui(false)

func _update_ui_on_files_saved():
	print("update_ui_on_files_saved")
	update_ui(true)

func _update_ui_on_files_changed():
	print("update_ui_on_files_changed")
	update_ui(true)

func _update_ui_on_branch_checked_out(_branch):
	print("update_ui_on_branch_checked_out")
	update_ui(true)

func _on_sync_server_connection_info_changed(_peer_connection_info: Dictionary) -> void:
	update_ui(false)

func _on_initial_checked_out_branch(_branch):
	print("on_initial_checked_out_branch")
	GodotProject.disconnect("checked_out_branch", self._on_initial_checked_out_branch)
	init()

func _on_reload_ui_button_pressed():
	reload_ui.emit()

# TODO: It seems that Sidebar is being instantiated by the editor before the plugin does?
func _ready() -> void:
	print("Sidebar: ready!")
	%ReloadUIButton.pressed.connect(self._on_reload_ui_button_pressed)
	# need to add task_modal as a child to the plugin otherwise process won't be called
	add_child(task_modal)
	# The singleton class accessor is still pointing to the old GodotProject singleton
	# if we're hot-reloading, so we check the Engine for the singleton instead.
	# The rest of the accessor uses outside of _ready() should be fine.
	var godot_project = Engine.get_singleton("GodotProject")
	if godot_project:
		if not godot_project.get_checked_out_branch():
			godot_project.connect("checked_out_branch", self._on_initial_checked_out_branch)
			task_modal.start_task("Loading Patchwork")
		else:
			init()
	else:
		print("!!!!!!GodotProject not initialized!")


func _process(delta: float) -> void:
	if deterred_highlight_update:
		var c = deterred_highlight_update
		deterred_highlight_update = null
		c.call()

	if waiting_callables.size() > 0:
		var callables = waiting_callables.duplicate()
		for callable in callables:
			callable.call()
		waiting_callables.clear()

func init() -> void:
	print("Sidebar initialized!")
	task_modal.end_task("Loading Patchwork")
	update_ui(true)

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

	sync_status_icon.pressed.connect(_on_sync_status_icon_pressed)

	history_section_header.pressed.connect(func(): toggle_section(history_section_header, history_section_body))
	diff_section_header.pressed.connect(func(): toggle_section(diff_section_header, diff_section_body))
	history_list.item_clicked.connect(_on_history_list_item_selected)
	history_list.empty_clicked.connect(_on_empty_clicked)
	inspector.node_hovered.connect(_on_node_hovered)
	inspector.node_unhovered.connect(_on_node_unhovered)

func _on_sync_status_icon_pressed():
	var sync_info = GodotProject.get_sync_server_connection_info()
	var checked_out_branch = GodotProject.get_checked_out_branch()

	print("Sync info ===========================", )
	print("is connected: ", sync_info.is_connected)
	print("last received: ", human_readable_timestamp(sync_info.last_received * 1000.0))
	print("last sent: ", human_readable_timestamp(sync_info.last_sent * 1000.0))


	if checked_out_branch && sync_info.doc_sync_states.has(checked_out_branch.id):
		var doc_sync_state = sync_info.doc_sync_states[checked_out_branch.id]

		print(checked_out_branch.name, ":")
		print("  acked heads: ", doc_sync_state.last_acked_heads)
		print("  sent heads: ", doc_sync_state.last_sent_heads)
		if doc_sync_state.last_sent != null:
			print("  last sent: ", human_readable_timestamp(doc_sync_state.last_sent * 1000.0))
		else:
			print("  last sent: -")
		if doc_sync_state.last_received != null:
			print("  last received: ", human_readable_timestamp(doc_sync_state.last_received * 1000.0))
		else:
			print("  last received: -")

	print("=====================================", )



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

		update_ui(false)
		dialog.queue_free()
	)

	add_child(dialog)
	dialog.popup_centered()

func _on_branch_picker_item_selected(_index: int) -> void:
	var selected_branch = branch_picker.get_item_metadata(_index)

	# reset selection in branch picker in case checkout_branch fails
	# once branch is actually checked out, the branch picker will update
	update_ui(false)

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

	if not selected_branch:
		printerr("no selected branch")
		return

	checkout_branch(selected_branch.id)

func _on_highlight_changes_checkbox_toggled(pressed: bool) -> void:
	highlight_changes = pressed
	update_ui(true)

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
	if not checked_out_branch:
		printerr("no checked out branch")
		return

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
	var parent_vbox = section_header.get_parent()
	if section_body.visible:
		section_header.icon = load("res://addons/patchwork/icons/collapsable-closed.svg")
		section_body.visible = false
		parent_vbox.set_v_size_flags(Control.SIZE_FILL)
	else:
		section_header.icon = load("res://addons/patchwork/icons/collapsable-open.svg")
		section_body.visible = true
		parent_vbox.set_v_size_flags(Control.SIZE_EXPAND_FILL)

func unfold_section(section_header: Button, section_body: Control):
	section_header.icon = load("res://addons/patchwork/icons/collapsable-open.svg")
	section_body.visible = true

func fold_section(section_header: Button, section_body: Control):
	section_header.icon = load("res://addons/patchwork/icons/collapsable-closed.svg")
	section_body.visible = false

func update_ui(update_diff: bool = false) -> void:
	var checked_out_branch = GodotProject.get_checked_out_branch()
	var main_branch = GodotProject.get_main_branch()
	var all_branches = GodotProject.get_branches()

	# update branch pickers
	diff_section_header.text = "Changes"

	update_branch_picker(main_branch, checked_out_branch, all_branches)

	# update history

	var peer_connection_info = GodotProject.get_sync_server_connection_info()
	var history = GodotProject.get_changes()
	var unsynced_changes = get_unsynced_changes(peer_connection_info, checked_out_branch, history)


	history_list.clear()

	for i in range(history.size() - 1, -1, -1):
		var change = history[i]

		if !("branch_id" in change) || change.branch_id != checked_out_branch.id:
			continue

		var change_author
		if "username" in change:
			change_author = change.username
		else:
			change_author = "Anonymous"

		var change_timestamp = human_readable_timestamp(change.timestamp)

		var prefix = ""

		if DEBUG_MODE:
			prefix = change.hash.substr(0, 8) + " - "

		if "merge_metadata" in change:
			var merged_branch = GodotProject.get_branch_by_id(change.merge_metadata.merged_branch_id)
			var merged_branch_name = merged_branch.name
			history_list.add_item(prefix + "↪️ " + change_author + " merged \"" + merged_branch_name + "\" branch - " + change_timestamp)
			history_list.set_item_metadata(history_list.get_item_count() - 1, change.hash)

		else:
			history_list.add_item(prefix + change_author + " made some changes - " + change_timestamp + "")
			history_list.set_item_metadata(history_list.get_item_count() - 1, change.hash)

		if unsynced_changes.has(change.hash):
			history_list.set_item_custom_fg_color(history_list.get_item_count() - 1, Color(0.5, 0.5, 0.5))


	# update sync status
	update_sync_status(peer_connection_info, checked_out_branch, history)

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

			if source_branch.forked_at != target_branch.heads:
				merge_preview_message_label.text = "\"" + target_branch.name + "\" has changed since \"" + source_branch.name + "\" was created.\nBe careful and review your changes before merging."
				merge_preview_message_icon.texture = load("res://addons/patchwork/icons/warning-circle.svg")
			else:
				merge_preview_message_label.text = "This branch is safe to merge.\n \"" + target_branch.name + "\" hasn't changed since \"" + source_branch.name + "\" was created."
				merge_preview_message_icon.texture = load("res://addons/patchwork/icons/checkmark-circle.svg")

	else:
		move_inspector_to_main()

	# DIFF

	if !update_diff:
		return

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


		# print("heads_before: ", heads_before)
		# print("heads_after: ", heads_after)

		var diff = update_properties_diff(checked_out_branch, history, heads_before, heads_after)

		inspector.visible = true


		update_highlight_changes(diff, checked_out_branch)

func update_sync_status(peer_connection_info, checked_out_branch, changes) -> void:
	if !checked_out_branch:
		return

	if !peer_connection_info:
		printerr("no peer connection info")
		return

	# unknown sync status
	if !peer_connection_info.doc_sync_states.has(checked_out_branch.id):
		sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-alert.svg")
		sync_status_icon.tooltip_text = "Disconnected - might have unsynced changes"
		return

	var sync_status = peer_connection_info.doc_sync_states[checked_out_branch.id];

	# fully synced
	if sync_status.last_acked_heads == checked_out_branch.heads:
		if peer_connection_info.is_connected:
			sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-check.svg")
			sync_status_icon.tooltip_text = "Fully synced"
		else:
			sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-alert.svg")
			sync_status_icon.tooltip_text = "Disconnected - no unsynced local changes"
		return

	# partially synced
	if peer_connection_info.is_connected:
		sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-sync.svg")
		sync_status_icon.tooltip_text = "Syncing"
	else:
		sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-alert.svg")

		var unsynced_changes = get_unsynced_changes(peer_connection_info, checked_out_branch, changes)
		var unsynced_changes_count = unsynced_changes.size()

		if unsynced_changes_count == 1:
			sync_status_icon.tooltip_text = "Disconnected - 1 local change that hasn't been synced"
		else:
			sync_status_icon.tooltip_text = "Disconnected - %s local changes that haven't been synced" % [unsynced_changes_count]


func get_unsynced_changes(connection_info, checked_out_branch, changes):
	var dict = {}

	if not checked_out_branch:
		printerr("no checked out branch")
		return

	for change in changes:
		dict[change.hash] = true


	if !connection_info:
		return dict

	var doc_sync_states = connection_info.doc_sync_states

	if !doc_sync_states:
		return dict

	if !(checked_out_branch.id in doc_sync_states):
		return dict

	var sync_status = doc_sync_states[checked_out_branch.id]

	if !sync_status:
		return dict


	var synced_until_index = -1
	for i in range(changes.size()):
		var change = changes[i]
		if sync_status.last_acked_heads.has(change.hash):
			synced_until_index = i
			break

	if synced_until_index == -1:
		return dict

	for i in range(synced_until_index + 1):
		dict.erase(changes[i].hash)

	return dict

func update_branch_picker(main_branch, checked_out_branch, all_branches) -> void:
	branch_picker.clear()

	if !checked_out_branch:
		return

	if !main_branch:
		return

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

func update_highlight_changes(diff: Dictionary, checked_out_branch: Dictionary, force_highlight: bool = false) -> void:
	if (PatchworkEditor.is_changing_scene()):
		deterred_highlight_update = func(): update_highlight_changes(diff, checked_out_branch)
		return

	var edited_root = EditorInterface.get_edited_scene_root()

	# reflect highlight changes checkbox state
	highlight_changes_checkbox_mp.button_pressed = highlight_changes
	highlight_changes_checkbox.button_pressed = highlight_changes

	if edited_root:
		if (highlight_changes || force_highlight) && !checked_out_branch.is_main:
				var path = edited_root.scene_file_path
				var scene_changes = diff.get(path)
				if scene_changes:
					HighlightChangesLayer.highlight_changes(edited_root, scene_changes)
		else:
			HighlightChangesLayer.remove_highlight(edited_root)


var prev_heads_before
var prev_heads_after
var last_diff: Dictionary = {}


func _on_node_hovered(file_path: String, node_path: NodePath) -> void:
	# print("on_node_hovered: ", file_path, node_path)
	var node: Node = EditorInterface.get_edited_scene_root()
	if node.scene_file_path != file_path:
		# don't highlight changes for other files
		return
	var lst_diff = last_diff
	# create a diff that only contains the changes for the hovered node
	for file in lst_diff.keys():
		if file == file_path:
			var diff: Dictionary = lst_diff[file].duplicate()
			var scene_changes = diff["changed_nodes"].duplicate()
			var new_scene_changes = []
			for node_change in scene_changes:
				var np: String = node_change["node_path"]
				if np == String(node_path) || np == "./" + String(node_path):
					new_scene_changes.append(node_change)
					break
			diff["changed_nodes"] = new_scene_changes
			lst_diff = {}
			lst_diff[file] = diff
			break
	# print("Updating highlight changes")
	self.update_highlight_changes(lst_diff, GodotProject.get_checked_out_branch(), true)

func _on_node_unhovered(file_path: String, node_path: NodePath) -> void:
	self.update_highlight_changes(last_diff, GodotProject.get_checked_out_branch(), false)

func _on_history_list_item_selected(index: int, _button, _modifiers) -> void:
	var change_hash = history_list.get_item_metadata(index)
	if change_hash:
		var change_heads = PackedStringArray([change_hash])
		# we're just updating the diff
		var checked_out_branch = GodotProject.get_checked_out_branch()
		# we show changes from most recent to oldest, so the previous change is the next index
		var prev_idx = index + 1
		var previous_heads: PackedStringArray = []
		if prev_idx >= history_list.get_item_count():
			# return
			# get the root hash from the checked_out_branch
			previous_heads = checked_out_branch.get("forked_at", PackedStringArray([]))
		else:
			previous_heads = [history_list.get_item_metadata(prev_idx)]

		if previous_heads.size() > 0:
			# diff_section_header.text = DIFF_SECTION_HEADER_TEXT_FORMAT % [prev_change_hash.substr(0, 7), change_hash.substr(0, 7)]
			var text = history_list.get_item_text(index)
			var name = text.split(" ")[0].strip_edges()
			if name == "↪️":
				name = text.split(" ")[1].strip_edges() + "'s merged branch"
			var date = text.split("-")[1].strip_edges()
			diff_section_header.text = "Showing changes from %s - %s" % [name, date]
			var diff = update_properties_diff(checked_out_branch, ["foo", "bar"], previous_heads, change_heads)
			inspector.visible = true
			update_highlight_changes(diff, checked_out_branch)
		else:
			printerr("no prev change hash")
	else:
		printerr("no change hash")

func _on_empty_clicked(_vec2, idx):
	update_ui(true)

func update_properties_diff(checked_out_branch, changes, heads_before, heads_after) -> Dictionary:

	if (!inspector):
		return last_diff
	if (!checked_out_branch):
		return last_diff

	if (changes.size() < 2):
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

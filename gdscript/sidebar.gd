@tool
extends MarginContainer
# This is a Godot 4.x script file, written in GDScript 2.0. Connections are made using the identifier for the callable directly.
# Godot 3.x: something.connect("signal_name", self, "_on_signal_name")
# Godot 4.x: something.connect("signal_name", self._on_signal_name)

const diff_inspector_script = preload("res://addons/patchwork/gdscript/diff_inspector_container.gd")
@onready var branch_picker: OptionButton = %BranchPicker
@onready var history_tree: Tree = %HistoryTree
@onready var user_button: Button = %UserButton
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

const DEV_MODE = true

# Turn this off if it keeps crashing on windows
const TURN_ON_USER_BRANCH_PROMPT = true

const DIFF_SECTION_HEADER_TEXT_FORMAT = "Changes: Showing diff between %s and %s"

const TEMP_DIR = "user://tmp"

var plugin: EditorPlugin

var task_modal: TaskModal = TaskModal.new()

var highlight_changes = false

var waiting_callables: Array = []

var deterred_highlight_update = null

var history_item_count = 0
var history_saved_selection = null # hash string

const CREATE_BRANCH_IDX = 1
const MERGE_BRANCH_IDX = 2

signal reload_ui();
signal user_name_initialized();


func _update_ui_on_branches_changed(_branches: Array):
	print("update_ui_on_branches_changed")
	var current_branch = GodotProject.get_checked_out_branch()
	var update_diff = false
	for branch in _branches:
		if branch.get("id", "") == current_branch.get("id", ""):
			update_diff = true
			break
	update_ui(update_diff)

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
	init(true)

func _on_reload_ui_button_pressed():
	reload_ui.emit()

func wait_for_checked_out_branch():
	var godot_project = Engine.get_singleton("GodotProject")
	if not godot_project.get_checked_out_branch():
		godot_project.connect("checked_out_branch", self._on_initial_checked_out_branch)
		task_modal.start_task("Loading Patchwork")
	else:
		init(false)



func start_and_wait_for_checkout():
	var godot_project = Engine.get_singleton("GodotProject")
	godot_project.start()
	make_init_button_invisible()
	wait_for_checked_out_branch()

func check_and_prompt_for_user_name(callback: Callable):
	for connection in user_name_initialized.get_connections():
		user_name_initialized.disconnect(connection.callable)
	var user_name = PatchworkConfig.get_user_value("user_name", "")
	if user_name.is_empty():
		user_name_initialized.connect(callback)
		_on_user_button_pressed(true)
		return false
	return true

func _on_init_button_pressed():
	if create_unsaved_files_dialog("Please save your unsaved files before initializing a new project."):
		return
	if not check_and_prompt_for_user_name(self._on_init_button_pressed):
		return
	print("Initializing new project!")
	start_and_wait_for_checkout()

func _on_load_project_button_pressed():
	if create_unsaved_files_dialog("Please save your unsaved files before loading an existing project."):
		return
	if not check_and_prompt_for_user_name(self._on_load_project_button_pressed):
		return
	var doc_id = %ProjectIDBox.text.strip_edges()
	if doc_id.is_empty():
		popup_box(self, $ErrorDialog, "Project ID is empty", "Error")
		return
	print("Loading project ", doc_id)
	PatchworkConfig.set_project_value("project_doc_id", doc_id)
	start_and_wait_for_checkout()

func add_listener_disable_button_if_text_is_empty(button: Button, line_edit: LineEdit):
	var listener = func(new_text: String):
		button.disabled = new_text.strip_edges().is_empty()
	line_edit.text_changed.connect(listener)
	listener.call(line_edit.text)

func make_init_button_visible():
	%InitPanelContainer.visible = true
	%MainVSplit.visible = false

func make_init_button_invisible():
	%InitPanelContainer.visible = false
	%MainVSplit.visible = true

func get_doc_id() -> String:
	var patchwork_config = Engine.get_singleton("PatchworkConfig")
	return patchwork_config.get_project_value("project_doc_id", "")

func _clear_user_name_initialized_connections():
	for connection in user_name_initialized.get_connections():
		user_name_initialized.disconnect(connection.callable)

func _on_user_name_confirmed():
	if %UserNameEntry.text.strip_edges() != "":
		var current_name = PatchworkConfig.get_user_value("user_name", "")
		var new_user_name = %UserNameEntry.text.strip_edges()
		print(new_user_name)
		PatchworkConfig.set_user_value("user_name", new_user_name)
		GodotProject.set_user_name(new_user_name)
		if current_name.is_empty():
			user_name_initialized.emit()
			call_deferred("_clear_user_name_initialized_connections")
	update_ui(false)

func _on_user_button_pressed(disable_cancel: bool = false):
	%UserNameEntry.text = PatchworkConfig.get_user_value("user_name", "")
	%UserNameDialog.popup_centered()
	%UserNameDialog.get_cancel_button().visible = not disable_cancel

func _on_clear_project_button_pressed():
	popup_box(self, $ConfirmationDialog, "Are you sure you want to clear the project?", "Clear Project", func(): clear_project())

func clear_project():
	GodotProject.stop()
	PatchworkConfig.set_user_value("user_name", "")
	PatchworkConfig.set_project_value("project_doc_id", "")
	PatchworkConfig.set_project_value("checked_out_branch_doc_id", "")
	_on_reload_ui_button_pressed()

# TODO: It seems that Sidebar is being instantiated by the editor before the plugin does?
func _ready() -> void:
	print("Sidebar: ready!")
	%ReloadUIButton.pressed.connect(self._on_reload_ui_button_pressed)
	if DEV_MODE:
		%ClearProjectButton.visible = true
		%ClearProjectButton.pressed.connect(self._on_clear_project_button_pressed)
	else:
		%ClearProjectButton.visible = false
	%InitializeButton.pressed.connect(self._on_init_button_pressed)
	%LoadExistingButton.pressed.connect(self._on_load_project_button_pressed)
	add_listener_disable_button_if_text_is_empty(%UserNameDialog.get_ok_button(), %UserNameEntry)
	add_listener_disable_button_if_text_is_empty(%LoadExistingButton, %ProjectIDBox)
	user_button.pressed.connect(_on_user_button_pressed)
	history_tree.clear()
	branch_picker.clear()


	# need to add task_modal as a child to the plugin otherwise process won't be called
	add_child(task_modal)
	if not EditorInterface.get_edited_scene_root() == self:
		waiting_callables.append(self._try_init)
	else:
		print("Sidebar: in editor!!!!!!!!!!!!")

func _try_init():
	# The singleton class accessor is still pointing to the old GodotProject singleton
	# if we're hot-reloading, so we check the Engine for the singleton instead.
	# The rest of the accessor uses outside of _ready() should be fine.
	var godot_project = Engine.get_singleton("GodotProject")
	if godot_project:
		var doc_id = get_doc_id()
		if not godot_project.is_started() and doc_id.is_empty():
			print("Not initialized, showing init button")
			make_init_button_visible()
			return
		else:
			print("Initialized, hiding init button")
			make_init_button_invisible()
			wait_for_checked_out_branch()
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

func _check_for_user_branch():
	var all_branches = GodotProject.get_branches()
	var user_name = PatchworkConfig.get_user_value("user_name", "")
	var has_user_branch = false
	for branch in all_branches:
		if not branch.is_main and branch.has("created_by") and branch.created_by == PatchworkConfig.get_user_value("user_name", ""):
			has_user_branch = true
			break
	if not has_user_branch:
		create_new_branch(true)


func init(end_task: bool = true) -> void:
	print("Sidebar initialized!")
	if end_task:
		task_modal.end_task("Loading Patchwork")
	branch_picker.disabled = false
	fork_button.disabled = false
	%CopyProjectIDButton.disabled = false
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
	%ClearDiffButton.pressed.connect(_on_clear_diff_button_pressed)

	branch_picker.item_selected.connect(_on_branch_picker_item_selected)

	cancel_merge_button.pressed.connect(cancel_merge_preview)
	confirm_merge_button.pressed.connect(confirm_merge_preview)

	sync_status_icon.pressed.connect(_on_sync_status_icon_pressed)

	history_section_header.pressed.connect(func(): toggle_section(history_section_header, history_section_body))
	diff_section_header.pressed.connect(func(): toggle_section(diff_section_header, diff_section_body))
	history_tree.item_selected.connect(_on_history_list_item_selected)
	history_tree.button_clicked.connect(_on_history_tree_button_clicked)
	history_tree.empty_clicked.connect(_on_empty_clicked)
	inspector.node_hovered.connect(_on_node_hovered)
	inspector.node_unhovered.connect(_on_node_unhovered)

	if not check_and_prompt_for_user_name(self._check_for_user_branch):
		return

	if not TURN_ON_USER_BRANCH_PROMPT:
		return
	var timeout = 5.0
	var timer = Timer.new()
	timer.wait_time = timeout
	timer.one_shot = true
	timer.timeout.connect(self._check_for_user_branch)
	add_child(timer)
	timer.start()

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

func create_unsaved_files_dialog(message: String):
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
		return true
	return false


func ensure_user_has_no_unsaved_files(message: String, callback: Callable):
	# todo: add back auto save
	if create_unsaved_files_dialog(message):
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

func create_new_branch(disable_cancel: bool = false) -> void:
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

		dialog.get_cancel_button().visible = not disable_cancel

		dialog.get_ok_button().text = "Create"
		add_listener_disable_button_if_text_is_empty(dialog.get_ok_button(), branch_name_input)

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

func update_history_ui(checked_out_branch, main_branch, all_branches, history, peer_connection_info):
	var unsynced_changes = get_unsynced_changes(peer_connection_info, checked_out_branch, history)

	history_tree.clear()
	history_item_count = 0

	# create root item
	var root = history_tree.create_item()
	var selection = null

	for i in range(history.size() - 1, -1, -1):
		var change = history[i]

		if !("branch_id" in change) || change.branch_id != checked_out_branch.id:
			continue

		var change_author
		if "username" in change:
			change_author = change.username
		else:
			change_author = "Anonymous"

		var item = history_tree.create_item(root)
		history_item_count += 1

		# set columns
		var column_index = 0
		var base_column_count = 2

		# if we're a dev, we need another column for the commit hash
		if DEV_MODE:
			history_tree.columns = base_column_count + 1
			item.set_text(column_index, change.hash.substr(0, 8))
			item.set_tooltip_text(column_index, change.hash)
			item.set_selectable(0, false)
			history_tree.set_column_expand(0, true)
			history_tree.set_column_expand_ratio(0, 0)
			history_tree.set_column_custom_minimum_width(0, 80)
			column_index += 1
		else:
			history_tree.columns = base_column_count

		item.set_metadata(0, change.hash)
		history_tree.set_column_expand(column_index, true)
		history_tree.set_column_expand_ratio(column_index, 2)
		item.set_selectable(column_index, true)

		if "merge_metadata" in change:
			var merged_branch = GodotProject.get_branch_by_id(change.merge_metadata.merged_branch_id)
			var merged_branch_name = str(change.merge_metadata.merged_branch_id)
			if merged_branch:
				merged_branch_name = merged_branch.name
			item.set_text(column_index, "↪️ " + change_author + " merged \"" + merged_branch_name + "\" branch")
			item.add_button(column_index, load("res://addons/patchwork/icons/branch-icon-history.svg"), 0, false, "Checkout branch " + merged_branch_name)

		else:
			item.set_text(column_index, change_author + " made some changes")

		column_index += 1;
		# timestamp
		item.set_text(column_index, human_readable_timestamp(change.timestamp))
		item.set_tooltip_text(column_index, exact_human_readable_timestamp(change.timestamp))
		item.set_selectable(column_index, false)
		history_tree.set_column_expand(column_index, true)
		history_tree.set_column_expand_ratio(column_index, 0)
		history_tree.set_column_custom_minimum_width(column_index, 150)

		if unsynced_changes.has(change.hash):
			item.set_custom_color(0, Color(0.5, 0.5, 0.5))

		if change.hash == history_saved_selection:
			selection = item

	# restore saved selection
	if selection != null:
		history_tree.set_selected(selection, 0)
	# otherwise, ensure any invalid saved selection is reset
	else:
		history_saved_selection = null

func update_ui(update_diff: bool = false) -> void:
	var checked_out_branch = GodotProject.get_checked_out_branch()
	var main_branch = GodotProject.get_main_branch()
	var all_branches = GodotProject.get_branches()
	var history = GodotProject.get_changes()
	var peer_connection_info = GodotProject.get_sync_server_connection_info()

	# update branch pickers
	update_branch_picker(main_branch, checked_out_branch, all_branches)

	# update the history tree
	update_history_ui(checked_out_branch, main_branch, all_branches, history, peer_connection_info)

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

	var source_branch = GodotProject.get_branch_by_id(checked_out_branch.forked_from) if checked_out_branch.has("forked_from") else null
	if checked_out_branch.is_merge_preview:
		move_inspector_to_merge_preview()
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
		if source_branch:
			diff_section_header.text = "Showing changes from \"" + source_branch.name + "\" -> \"" + checked_out_branch.name + "\""

	if update_diff: update_diff()

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

func exact_human_readable_timestamp(timestamp: int) -> String:
	return Time.get_datetime_string_from_unix_time(round(timestamp / 1000.0)) + " UTC"

func update_highlight_changes(diff: Dictionary) -> void:
	if (PatchworkEditor.is_changing_scene()):
		deterred_highlight_update = func(): update_highlight_changes(diff)
		return

	var edited_root = EditorInterface.get_edited_scene_root()

	# reflect highlight changes checkbox state

	if edited_root:
		if not (not diff || diff.is_empty()):
				var path = edited_root.scene_file_path
				var scene_changes = diff.get(path)
				if scene_changes:
					HighlightChangesLayer.highlight_changes(edited_root, scene_changes)
		else:
			HighlightChangesLayer.remove_highlight(edited_root)


var prev_heads_before
var prev_heads_after
var last_diff: Dictionary = {}


func _on_node_hovered(file_path: String, node_paths: Array) -> void:
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
				if node_paths.has(NodePath(np)):
					new_scene_changes.append(node_change)
			diff["changed_nodes"] = new_scene_changes
			lst_diff = {}
			lst_diff[file] = diff
			break
	# print("Updating highlight changes")
	self.update_highlight_changes(lst_diff)

func _on_node_unhovered(file_path: String, node_path: Array) -> void:
	self.update_highlight_changes({})

func _on_history_tree_button_clicked(item: TreeItem, column : int, id: int, mouse_button_index: int) -> void:
	if mouse_button_index != 1: return
	var change_hash = item.get_metadata(0)
	var history = GodotProject.get_changes()

	history = history.filter(func (change): return change.hash == change_hash);
	var change = history[0] if not history.is_empty() else null
	if change == null:
		print("Error: No matching change found.")
		return;

	var merged_branch = GodotProject.get_branch_by_id(change.merge_metadata.merged_branch_id)
	checkout_branch(merged_branch.id)

func _on_history_list_item_selected() -> void:
	var selected_item = history_tree.get_selected()
	if selected_item == null:
		history_saved_selection = null
		return

	# update the saved selection
	var change_hash = selected_item.get_metadata(0)
	history_saved_selection = change_hash

	update_diff()

func _on_clear_diff_button_pressed():
	_on_empty_clicked(null, 0)

func _on_empty_clicked(_vec2, _idx):
	history_saved_selection = null
	history_tree.deselect_all()
	update_diff()

# read the selection from the tree, and update the diff visualization accordingly.
func update_diff():
	var selected_item = history_tree.get_selected()
	var checked_out_branch = GodotProject.get_checked_out_branch()

	# check to see if we generate a selection diff, or a regular diff
	if (selected_item == null
			or checked_out_branch.is_merge_preview
			# the first three commits on main are initial checkin, so we don't show a diff for them
			or checked_out_branch.is_main
				and selected_item.get_index() >= history_item_count - 3):
		update_diff_default(checked_out_branch, GodotProject.get_changes().size())
		return

	# otherwise, we set up the diff between the selected commit and the previous.
	var change_hash = selected_item.get_metadata(0)
	if change_hash:
		var change_heads = PackedStringArray([change_hash])
		# we're just updating the diff
		# we show changes from most recent to oldest, so the previous change is the next item
		var prev_item = selected_item.get_next_in_tree()
		var previous_heads: PackedStringArray = []
		if prev_item == null:
			if checked_out_branch.is_main:
				return
			# get the root hash from the checked_out_branch
			previous_heads = checked_out_branch.get("forked_at", PackedStringArray([]))
		else:
			previous_heads = [prev_item.get_metadata(0)]

		if previous_heads.size() > 0:
			# diff_section_header.text = DIFF_SECTION_HEADER_TEXT_FORMAT % [prev_change_hash.substr(0, 7), change_hash.substr(0, 7)]
			var text = selected_item.get_text(1 if DEV_MODE else 0)
			var date = selected_item.get_text(2 if DEV_MODE else 1)
			var name = text.split(" ")[0].strip_edges()
			if name == "↪️":
				name = text.split(" ")[1].strip_edges() + "'s merged branch"
			diff_section_header.text = "Showing changes from %s - %s" % [name, date]
			var diff = update_properties_diff(checked_out_branch, 2, previous_heads, change_heads)
			%ClearDiffButton.visible = true
			inspector.visible = true
		else:
			printerr("no prev change hash")
	else:
		printerr("no change hash")

# display the default diff, for when there's no available selected diff or if we're merging
func update_diff_default(checked_out_branch, history):
	%ClearDiffButton.visible = false

	# show no diff for main branch
	if checked_out_branch.is_main:
		inspector.visible = false
		diff_section_header.text = "Changes"

	else:
		var heads_before
		var heads_after

		var source_branch = GodotProject.get_branch_by_id(checked_out_branch.forked_from)
		if checked_out_branch.is_merge_preview:
			var target_branch = GodotProject.get_branch_by_id(checked_out_branch.merge_into)
			heads_before = checked_out_branch.merge_at
			heads_after = checked_out_branch.heads
			diff_section_header.text = "Showing changes for \"" + source_branch.name + "\" -> \"" + target_branch.name + "\""

		else:
			heads_before = checked_out_branch.forked_at
			heads_after = checked_out_branch.heads
			diff_section_header.text = "Showing changes from \"" + source_branch.name + "\" -> \"" + checked_out_branch.name + "\""

		print("heads_before: ", heads_before)
		print("heads_after: ", heads_after)

		var diff = update_properties_diff(checked_out_branch, history, heads_before, heads_after)

		inspector.visible = true

func update_properties_diff(checked_out_branch, change_count, heads_before, heads_after) -> Dictionary:

	if (!inspector):
		return last_diff
	if (!checked_out_branch):
		return last_diff

	if (change_count < 2):
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


func _on_copy_project_id_button_pressed() -> void:
	var project_id = PatchworkConfig.get_project_value("project_doc_id", "")
	if not project_id.is_empty():
		DisplayServer.clipboard_set(project_id)

@tool
extends MarginContainer
# This is a Godot 4.x script file, written in GDScript 2.0. Connections are made using the identifier for the callable directly.
# Godot 3.x: something.connect("signal_name", self, "_on_signal_name")
# Godot 4.x: something.connect("signal_name", self._on_signal_name)

const diff_inspector_script = preload("res://addons/patchwork/gdscript/diff_inspector_container.gd")
@onready var branch_picker: OptionButton = %BranchPicker
@onready var history_tree: Tree = %HistoryTree
@onready var history_list_popup: PopupMenu = %HistoryListPopup
@onready var user_button: Button = %UserButton
@onready var inspector: DiffInspectorContainer = %BigDiffer
@onready var merge_preview_modal: Control = %MergePreviewModal
@onready var cancel_merge_button: Button = %CancelMergeButton
@onready var confirm_merge_button: Button = %ConfirmMergeButton
@onready var merge_preview_title: Label = %MergePreviewTitle

@onready var merge_preview_source_label: Label = %MergePreviewSourceLabel
@onready var merge_preview_target_label: Label = %MergePreviewTargetLabel
@onready var merge_preview_diff_container: MarginContainer = %MergePreviewDiffContainer
@onready var revert_preview_modal: Control = %RevertPreviewModal
@onready var cancel_revert_button: Button = %CancelRevertButton
@onready var confirm_revert_button: Button = %ConfirmRevertButton
@onready var revert_preview_title: Label = %RevertPreviewTitle
@onready var revert_preview_message_label: Label = %RevertPreviewMessageLabel
@onready var revert_preview_message_icon: TextureRect = %RevertPreviewMessageIcon
@onready var revert_preview_diff_container: MarginContainer = %RevertPreviewDiffContainer
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

# Defines the column indices for the history tree.
class HistoryColumns:
	const HASH = 0 if DEV_MODE else -1
	const TEXT = 1 if DEV_MODE else 0
	const TIME = 2 if DEV_MODE else 1
	const COUNT = 3 if DEV_MODE else 2
	const HASH_META = 0
	const ENABLED_META = 1

const INITIAL_COMMIT_TEXT = "Initialized repository"

const NUM_INITIAL_COMMITS = 2

const DIFF_SECTION_HEADER_TEXT_FORMAT = "Changes: Showing diff between %s and %s"

const TEMP_DIR = "user://tmp"

var plugin: EditorPlugin
var task_modal: TaskModal = TaskModal.new()
var item_context_menu_icon: Texture2D = preload("../icons/GuiTabMenuHl_rotated.svg")
var highlight_changes = false
var waiting_callables: Array = []
var deferred_highlight_update = null

var all_changes_count = 0
var history_item_count = 0
var history_saved_selection = null # hash string

const CREATE_BRANCH_IDX = 1
const MERGE_BRANCH_IDX = 2

signal reload_ui();
signal user_name_dialog_closed();

func _update_ui_on_state_change():
	print("Patchwork: Updating UI due to state change...")
	update_ui()

func _update_ui_on_branch_checked_out():
	print("Patchwork: Updating UI due to branch checked out...")
	update_ui()

func _on_reload_ui_button_pressed():
	reload_ui.emit()

# Display a "Loading Patchwork" modal until we receive a checked_out_branch signal, then initialize.
# Used when creating a new project, manually loading an existing project from ID, or auto-loading
# an existing project from the project.
func wait_for_checked_out_branch():
	if not GodotProject.get_checked_out_branch():
		task_modal.start_task("Loading Patchwork")
		await GodotProject.checked_out_branch
		task_modal.end_task("Loading Patchwork")
	init()

# Asks the user for their username, if there is none stored.
# If they cancel or close, returns false. If the username is confirmed, returns true.
func require_user_name() -> bool:
	if !GodotProject.has_user_name():
		_on_user_button_pressed(true)
		await user_name_dialog_closed
		return GodotProject.has_user_name()
	return true

func _on_init_button_pressed():
	if create_unsaved_files_dialog("Please save your unsaved files before initializing a new project."):
		return
	if not await require_user_name():
		return

	GodotProject.new_project();
	await wait_for_checked_out_branch()

func _on_load_project_button_pressed():
	if create_unsaved_files_dialog("Please save your unsaved files before loading an existing project."):
		return
	var doc_id = %ProjectIDBox.text.strip_edges()
	if doc_id.is_empty():
		Utils.popup_box(self, $ErrorDialog, "Project ID is empty", "Error")
		return
	if not await require_user_name():
		return

	GodotProject.load_project(doc_id);
	await wait_for_checked_out_branch()

func update_init_panel():
	var visible = !GodotProject.has_project()
	%InitPanelContainer.visible = visible
	%MainVSplit.visible = !visible
	branch_picker.disabled = visible
	fork_button.disabled = visible
	%CopyProjectIDButton.disabled = visible

func _on_user_button_pressed(disable_cancel: bool = false):
	%UserNameEntry.text = GodotProject.get_user_name()
	%UserNameDialog.popup_centered()
	%UserNameDialog.get_cancel_button().visible = not disable_cancel

func _on_user_name_canceled():
	user_name_dialog_closed.emit()

func _on_user_name_confirmed():
	var new_user_name = %UserNameEntry.text.strip_edges()
	if new_user_name != "": GodotProject.set_user_name(new_user_name)
	user_name_dialog_closed.emit()
	print("Patchwork: Updating UI due to username confirmation...")
	update_ui()

func _on_clear_project_button_pressed():
	Utils.popup_box(self, $ConfirmationDialog, "Are you sure you want to clear the project?", "Clear Project",
		func(): clear_project(), func(): pass)

func clear_project():
	GodotProject.clear_project()
	_on_reload_ui_button_pressed()

func get_history_item_enabled(item: TreeItem) -> bool:
	return item.get_metadata(HistoryColumns.ENABLED_META)

func set_history_item_enabled(item: TreeItem, value: bool) -> void:
	item.set_metadata(HistoryColumns.ENABLED_META, value)

func get_history_item_hash(item: TreeItem) -> String:
	return item.get_metadata(HistoryColumns.HASH_META)

func set_history_item_hash(item: TreeItem, value: String) -> void:
	item.set_metadata(HistoryColumns.HASH_META, value)

# TODO: It seems that Sidebar is being instantiated by the editor before the plugin does?
func _ready() -> void:
	# @Paul: I think somewhere besides the plugin sidebar gets instantiated. Is this something godot does?
	# to paper over this we check if plugin and godot_project are set
	# The singleton class accessor is still pointing to the old GodotProject singleton
	# if we're hot-reloading, so we check the Engine for the singleton instead.
	# The rest of the accessor uses outside of _ready() should be fine.
	var godot_project = Engine.get_singleton("GodotProject")
	if !godot_project: return

	bind_listeners(godot_project)
	setup_history_list_popup()

	print("Sidebar: ready!")

	# need to add task_modal as a child to the plugin otherwise process won't be called
	add_child(task_modal)
	if not EditorInterface.get_edited_scene_root() == self:
		waiting_callables.append(self._try_init)
	else:
		print("Sidebar: in editor!!!!!!!!!!!!")

func bind_listeners(godot_project):
	%ReloadUIButton.pressed.connect(self._on_reload_ui_button_pressed)
	if DEV_MODE:
		%ClearProjectButton.visible = true
		%ClearProjectButton.pressed.connect(self._on_clear_project_button_pressed)
	else:
		%ClearProjectButton.visible = false
	%InitializeButton.pressed.connect(self._on_init_button_pressed)
	%LoadExistingButton.pressed.connect(self._on_load_project_button_pressed)
	Utils.add_listener_disable_button_if_text_is_empty(%UserNameDialog.get_ok_button(), %UserNameEntry)
	Utils.add_listener_disable_button_if_text_is_empty(%LoadExistingButton, %ProjectIDBox)
	user_button.pressed.connect(_on_user_button_pressed)

	%UserNameDialog.canceled.connect(_on_user_name_canceled)
	%UserNameDialog.confirmed.connect(_on_user_name_confirmed)

	godot_project.state_changed.connect(self._update_ui_on_state_change);
	godot_project.checked_out_branch.connect(self._update_ui_on_branch_checked_out);

	merge_button.pressed.connect(create_merge_preview_branch)
	fork_button.pressed.connect(create_new_branch)
	%ClearDiffButton.pressed.connect(_on_clear_diff_button_pressed)

	branch_picker.item_selected.connect(_on_branch_picker_item_selected)

	cancel_merge_button.pressed.connect(cancel_merge_preview)
	confirm_merge_button.pressed.connect(confirm_merge_preview)

	cancel_revert_button.pressed.connect(cancel_revert_preview)
	confirm_revert_button.pressed.connect(confirm_revert_preview)

	sync_status_icon.pressed.connect(_on_sync_status_icon_pressed)

	history_section_header.pressed.connect(func(): toggle_section(history_section_header, history_section_body))
	diff_section_header.pressed.connect(func(): toggle_section(diff_section_header, diff_section_body))
	history_tree.item_selected.connect(_on_history_list_item_selected)
	history_tree.button_clicked.connect(_on_history_tree_button_clicked)
	history_tree.empty_clicked.connect(_on_history_tree_empty_clicked)
	history_tree.item_mouse_selected.connect(_on_history_tree_mouse_selected)
	history_tree.allow_rmb_select = true
	inspector.node_hovered.connect(_on_node_hovered)
	inspector.node_unhovered.connect(_on_node_unhovered)
	%CopyProjectIDButton.pressed.connect(_on_copy_project_id_button_pressed)

func _try_init():
	var godot_project = Engine.get_singleton("GodotProject")
	if godot_project:
		if !godot_project.has_project():
			print("Not initialized, showing init panel")
			print("Patchwork: Updating UI due to init...")
			update_ui()
			return
		else:
			print("Initialized, hiding init panel")
			wait_for_checked_out_branch()
	else:
		print("No GodotProject singleton!!!!!!!!")

func _process(delta: float) -> void:
	if deferred_highlight_update:
		var c = deferred_highlight_update
		deferred_highlight_update = null
		c.call()

	if waiting_callables.size() > 0:
		var callables = waiting_callables.duplicate()
		for callable in callables:
			callable.call()
		waiting_callables.clear()

func init() -> void:
	print("Sidebar initialized!")
	print("Patchwork: Updating UI due to init...")
	update_ui()

	# Here, the user could easily just hit X and remain anonymous. This can only happen in the case
	# of a project loaded from a file, where the user's config hasn't been set.
	# If we want to force the user to enter a username, we could do `while(!require_user_name()): pass`.
	# But that seems bad.
	require_user_name()

func _on_sync_status_icon_pressed():
	GodotProject.print_sync_debug()

func _on_branch_picker_item_selected(_index: int) -> void:
	var selected_branch = GodotProject.get_branch(branch_picker.get_item_metadata(_index))

	# reset selection in branch picker in case checkout_branch fails
	# once branch is actually checked out, the branch picker will update
	print("Patchwork: Updating UI due to branch picker selection...")
	update_ui()

	if !selected_branch.is_loaded:
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

func checkout_branch(branch_id: String) -> void:
	var branch = GodotProject.get_branch(branch_id)
	if (!branch):
		Utils.popup_box(self, $ErrorDialog, "Branch not found", "Error")
		return

	if create_unsaved_files_dialog("You have unsaved files open. You need to save them before checking out another branch."):
		return;

	task_modal.do_task(
		"Checking out branch \"%s\"" % [branch.name],
		func():
			GodotProject.checkout_branch(branch_id)
			await GodotProject.checked_out_branch
	)

func create_new_branch() -> void:
	if create_unsaved_files_dialog("You have unsaved files open. You need to save them before creating a new branch."):
		return

	var dialog = ConfirmationDialog.new()
	dialog.title = "Create New Branch"

	var branch_name_input = LineEdit.new()
	branch_name_input.placeholder_text = "Branch name"
	branch_name_input.text = GodotProject.get_user_name() + "'s remix"
	dialog.add_child(branch_name_input)

	# Not scaling these values because they display correctly at 1x-2x scale
	# Position line edit in dialog
	branch_name_input.position = Vector2(8, 8)
	branch_name_input.size = Vector2(200, 30)

	# Make dialog big enough for line edit
	dialog.size = Vector2(220, 100)

	dialog.get_ok_button().text = "Create"
	Utils.add_listener_disable_button_if_text_is_empty(dialog.get_ok_button(), branch_name_input)

	dialog.canceled.connect(func(): dialog.queue_free())

	dialog.confirmed.connect(func():
		var new_branch_name = branch_name_input.text.strip_edges()
		dialog.queue_free()

		task_modal.do_task("Creating new branch \"%s\"" % new_branch_name, func():
			GodotProject.create_branch(new_branch_name)
			await GodotProject.checked_out_branch
		)
	)

	add_child(dialog)

	dialog.popup_centered()

	# focus on the branch name input
	branch_name_input.grab_focus()

func move_inspector_to(node: Node) -> void:
	if inspector and main_diff_container and node and inspector.get_parent() != node:
		inspector.reparent(node)
		inspector.visible = true

func create_merge_preview_branch():
	if create_unsaved_files_dialog("Please save your unsaved files before merging."):
		return

	# this shouldn't be possible due to UI disabling, but just in case
	if not GodotProject.can_create_merge_preview_branch():
		return

	task_modal.do_task("Creating merge preview", func():
		GodotProject.create_merge_preview_branch()
		await GodotProject.checked_out_branch
	)

func create_revert_preview_branch(head):
	if create_unsaved_files_dialog("Please save your unsaved files before reverting."):
		return
	# this shouldn't be possible due to UI disabling, but just in case
	if !GodotProject.can_create_revert_preview_branch(head): return

	task_modal.do_task("Creating revert preview", func():
		GodotProject.create_revert_preview_branch(head)
		await GodotProject.checked_out_branch
	)

func cancel_revert_preview():
	if !GodotProject.is_revert_preview_branch_active(): return
	task_modal.do_task("Cancel revert preview", func():
		GodotProject.discard_preview_branch()
		await GodotProject.checked_out_branch
	)

func confirm_revert_preview():
	if !GodotProject.is_revert_preview_branch_active(): return

	if create_unsaved_files_dialog("You have unsaved files open. You need to save them before reverting."):
		return

	var target = Utils.short_hash(GodotProject.get_checked_out_branch().reverted_to)

	Utils.popup_box(self, $ConfirmationDialog, "Are you sure you want to revert to \"%s\" ?" % target, "Revert Branch", func():
		task_modal.do_task("Reverting to \"%s\"" % target, func():
			GodotProject.confirm_preview_branch()
			await GodotProject.checked_out_branch
		), func(): pass)

func cancel_merge_preview():
	if !GodotProject.is_merge_preview_branch_active(): return
	task_modal.do_task("Cancel merge preview", func():
		GodotProject.discard_preview_branch()
		await GodotProject.checked_out_branch
	)


func confirm_merge_preview():
	if !GodotProject.is_merge_preview_branch_active(): return

	if create_unsaved_files_dialog("You have unsaved files open. You need to save them before merging."):
		return

	var current_branch = GodotProject.get_checked_out_branch()
	var forked_from = GodotProject.get_branch(current_branch.parent).name
	var target = GodotProject.get_branch(current_branch.merge_into).name

	Utils.popup_box(self, $ConfirmationDialog, "Are you sure you want to merge \"%s\" into \"%s\" ?" % [forked_from, target], "Merge Branch", func():
		task_modal.do_task("Merging \"%s\" into \"%s\"" % [forked_from, target], func():
			GodotProject.confirm_preview_branch()
			await GodotProject.checked_out_branch
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

func update_history_tree():
	if !GodotProject.has_project(): return
	var history = GodotProject.get_branch_history()

	history_tree.clear()
	history_item_count = 0

	# create root item
	var root = history_tree.create_item()
	var selection = null

	for i in range(history.size() - 1, -1, -1):
		var change = GodotProject.get_change(history[i])
		var item = history_tree.create_item(root)
		history_item_count += 1
		var editor_scale = EditorInterface.get_editor_scale()

		# if we're a dev, we need another column for the commit hash
		history_tree.columns = HistoryColumns.COUNT
		if DEV_MODE:
			item.set_text(HistoryColumns.HASH, Utils.short_hash(change.hash))
			item.set_tooltip_text(HistoryColumns.HASH, change.hash)
			item.set_selectable(HistoryColumns.HASH, false)
			history_tree.set_column_expand(HistoryColumns.HASH, true)
			history_tree.set_column_expand_ratio(HistoryColumns.HASH, 0)
			history_tree.set_column_custom_minimum_width(HistoryColumns.HASH, 80 * editor_scale)

		set_history_item_hash(item, change.hash)
		set_history_item_enabled(item, true)
		history_tree.set_column_expand(HistoryColumns.TEXT, true)
		history_tree.set_column_expand_ratio(HistoryColumns.TEXT, 2)
		item.set_selectable(HistoryColumns.TEXT, true)

		var text_color = Color.WHITE

		if change.is_merge:
			var merged_branch = GodotProject.get_branch(change.merge_id)
			item.add_button(HistoryColumns.TEXT, load("res://addons/patchwork/icons/branch-icon-history.svg"), 0,
				false, "Checkout branch " + merged_branch.name)

		item.set_text(HistoryColumns.TEXT, change.summary)

		if !change.is_synced:
			text_color = Color(0.6, 0.6, 0.6)

		# disable initial commits
		if change.is_setup:
			set_history_item_enabled(item, false);

		if change.is_synced && !change.is_setup:
			item.add_button(HistoryColumns.TEXT, item_context_menu_icon, 1, false, "Open context menu")

		# timestamp
		item.set_text(HistoryColumns.TIME, change.human_timestamp)
		item.set_tooltip_text(HistoryColumns.TIME, change.exact_timestamp)
		item.set_selectable(HistoryColumns.TIME, false)
		history_tree.set_column_expand(HistoryColumns.TIME, true)
		history_tree.set_column_expand_ratio(HistoryColumns.TIME, 0)
		history_tree.set_column_custom_minimum_width(HistoryColumns.TIME, 150 * editor_scale)

		# apply the chosen color to all fields
		item.set_custom_color(HistoryColumns.HASH, text_color)
		item.set_custom_color(HistoryColumns.TEXT, text_color)
		item.set_custom_color(HistoryColumns.TIME, text_color)

		if change.hash == history_saved_selection:
			selection = item

	# restore saved selection
	if selection != null:
		history_tree.set_selected(selection, 0)
	# otherwise, ensure any invalid saved selection is reset
	else:
		history_saved_selection = null

func update_action_buttons():
	if !GodotProject.has_project(): return
	var main_branch = GodotProject.get_main_branch()
	var current_branch = GodotProject.get_checked_out_branch()
	if !main_branch or !current_branch: return
	if main_branch.id == current_branch.id:
		merge_button.disabled = true
		merge_button.tooltip_text = "Can't merge main because it's not a remix of another branch"
	else:
		merge_button.disabled = false
		merge_button.tooltip_text = ""

func update_user_name():
	user_button.text = GodotProject.get_user_name()
	if user_button.text == "": user_button.text = "Anonymous"

func update_merge_preview():
	var active = GodotProject.is_merge_preview_branch_active()
	merge_preview_modal.visible = active
	if !active: return

	var current_branch = GodotProject.get_checked_out_branch()
	var source_branch = GodotProject.get_branch(current_branch.parent)
	var target_branch = GodotProject.get_branch(current_branch.merge_into)

	if !source_branch or !target_branch:
		printerr("Branch merge info invalid!")
		return;

	merge_preview_source_label.text = source_branch.name
	merge_preview_target_label.text = target_branch.name
	merge_preview_title.text = "Preview of \"" + target_branch.name + "\""

	if GodotProject.is_safe_to_merge():
		merge_preview_message_label.text = "\"" + target_branch.name + "\" has changed since \"" + source_branch.name + "\" was created.\nBe careful and review your changes before merging."
		merge_preview_message_icon.texture = load("res://addons/patchwork/icons/warning-circle.svg")
	else:
		merge_preview_message_label.text = "This branch is safe to merge.\n \"" + target_branch.name + "\" hasn't changed since \"" + source_branch.name + "\" was created."
		merge_preview_message_icon.texture = load("res://addons/patchwork/icons/checkmark-circle.svg")

func update_revert_preview():
	var active = GodotProject.is_revert_preview_branch_active()
	revert_preview_modal.visible = active
	if !active: return

	var current_branch = GodotProject.get_checked_out_branch()

	if !current_branch || !current_branch.reverted_to:
		printerr("Branch revert info invalid!")
		return

	var change_hash = Utils.short_hash(current_branch.reverted_to)

	revert_preview_title.text = "Preview of reverting to %s" % change_hash

func update_inspector():
	if !GodotProject.has_project(): return
	if GodotProject.is_revert_preview_branch_active():
		move_inspector_to(revert_preview_diff_container)
	elif GodotProject.is_merge_preview_branch_active():
		move_inspector_to(merge_preview_diff_container)
	else:
		move_inspector_to(main_diff_container)

# Refresh the entire UI, rebinding all data.
func update_ui() -> void:
	update_init_panel();
	update_branch_picker()
	update_history_tree()
	update_sync_status()
	update_action_buttons()
	update_user_name()
	update_inspector()
	update_revert_preview()
	update_merge_preview()
	update_diff()

func update_sync_status() -> void:
	var sync_status = GodotProject.get_sync_status()

	if sync_status.state == "unknown":
		sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-alert.svg")
		sync_status_icon.tooltip_text = "Disconnected - might have unsynced changes"

	elif sync_status.state == "syncing":
		sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-sync.svg")
		sync_status_icon.tooltip_text = "Syncing"

	elif sync_status.state == "up_to_date":
		sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-check.svg")
		sync_status_icon.tooltip_text = "Fully synced"

	elif sync_status.state == "disconnected":
		sync_status_icon.texture_normal = load("res://addons/patchwork/icons/circle-alert.svg")
		if sync_status.unsynced_changes == 0:
			sync_status_icon.tooltip_text = "Disconnected - no unsynced local changes"
		elif sync_status.unsynced_changes == 1:
			sync_status_icon.tooltip_text = "Disconnected - 1 local change that hasn't been synced"
		else:
			sync_status_icon.tooltip_text = "Disconnected - %s local changes that haven't been synced" % [sync_status.unsynced_changes]
	else: printerr("unknown sync status: " + sync_status.state)

# Update the branch selector.
func update_branch_picker() -> void:
	if !GodotProject.has_project(): return
	branch_picker.clear()

	var main_branch = GodotProject.get_main_branch();
	var checked_out_branch = GodotProject.get_checked_out_branch()
	if !checked_out_branch or !main_branch:
		return

	branch_picker_cover.text = checked_out_branch.name
	add_branch_to_picker(main_branch, checked_out_branch.id)

# Recursively add a branch and all of its child forks to the branch picker.
func add_branch_to_picker(branch: Dictionary, selected_branch_id: String, indentation: String = "", is_last: bool = false) -> void:
	if !branch.is_available: return

	var label
	if !branch.parent:
		label = branch.name
	else:
		var connection = "└─ " if is_last else "├─ "
		label = indentation + connection + branch.name

	var branch_index = branch_picker.get_item_count()
	branch_picker.add_item(label, branch_index)

	# this should not happen, but right now the sync is not working correctly so we need to surface this in the interface
	if !branch.is_loaded:
		branch_picker.set_item_icon(branch_index, load("res://addons/patchwork/icons/warning.svg"))

	branch_picker.set_item_metadata(branch_index, branch.id)

	if branch.id == selected_branch_id:
		branch_picker.select(branch_index)

	var new_indentation
	if !branch.parent:
		new_indentation = ""
	else:
		if is_last:
			new_indentation = indentation + "    "
		else:
			new_indentation = indentation + "│   "

	for i in range(branch.children.size()):
		var child = branch.children[i]
		var is_last_child = i == branch.children.size() - 1
		add_branch_to_picker(GodotProject.get_branch(child), selected_branch_id, new_indentation, is_last_child)

func update_highlight_changes(diff: Dictionary) -> void:
	if (PatchworkEditor.is_changing_scene()):
		deferred_highlight_update = func(): update_highlight_changes(diff)
		return

	var edited_root = EditorInterface.get_edited_scene_root()

	# reflect highlight changes checkbox state

	if edited_root:
		if not (not diff || diff.is_empty()):
			var path = edited_root.scene_file_path
			var scene_changes = diff.dict.get(path)
			if scene_changes:
				HighlightChangesLayer.highlight_changes(edited_root, scene_changes)
		else:
			HighlightChangesLayer.remove_highlight(edited_root)

var last_diff = null

func _on_node_hovered(file_path: String, node_paths: Array) -> void:
	var node: Node = EditorInterface.get_edited_scene_root()
	if node.scene_file_path != file_path or !last_diff:
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

var context_menu_hash = null

enum HistoryListPopupItem {
	RESET_TO_COMMIT,
	CREATE_BRANCH_AT_COMMIT
}

func _on_history_list_popup_id_pressed(index: int) -> void:
	history_list_popup.hide()
	var item = history_list_popup.get_item_id(index)
	if context_menu_hash == null:
		printerr("no selected item")
		return
	if item == HistoryListPopupItem.RESET_TO_COMMIT:
		create_revert_preview_branch(context_menu_hash)
	elif item == HistoryListPopupItem.CREATE_BRANCH_AT_COMMIT:
		print("Create remix at change not implemented yet!")

func setup_history_list_popup() -> void:
	history_list_popup.clear()
	# TODO: adjust this when more items are added
	history_list_popup.max_size.y = 48 * EditorInterface.get_editor_scale()
	history_list_popup.id_pressed.connect(_on_history_list_popup_id_pressed)
	history_list_popup.add_icon_item(load("res://addons/patchwork/icons/undo-redo.svg"), "Reset to here", HistoryListPopupItem.RESET_TO_COMMIT)
	# history_list_popup.add_item("Create remix from here", HistoryListPopupItem.CREATE_BRANCH_AT_COMMIT)

func _on_history_tree_mouse_selected(_at_position: Vector2, button_idx: int) -> void:
	if button_idx == MOUSE_BUTTON_RIGHT:
		# if the selected item is disabled, do not.
		if get_history_item_enabled(history_tree.get_selected()) == false: return
		show_contextmenu(get_history_item_hash(history_tree.get_selected()))

func show_contextmenu(item_hash):
	context_menu_hash = item_hash
	history_list_popup.position = DisplayServer.mouse_get_position()
	history_list_popup.visible = true

func _on_history_tree_button_clicked(item: TreeItem, _column : int, id: int, mouse_button_index: int) -> void:
	if mouse_button_index != MOUSE_BUTTON_LEFT: return

	if id == 0:
		var change_hash = get_history_item_hash(item)
		var change = GodotProject.get_change(change_hash)
		var merged_branch = change.merge_id
		if !merged_branch:
			print("Error: No matching change found.")
			return

		checkout_branch(merged_branch)
	elif id == 1:
		show_contextmenu(get_history_item_hash(item))

func _on_history_list_item_selected() -> void:
	var selected_item = history_tree.get_selected()
	if selected_item == null:
		history_saved_selection = null
		return

	# update the saved selection
	var change_hash = get_history_item_hash(selected_item)
	history_saved_selection = change_hash

	update_diff()

func _on_clear_diff_button_pressed():
	_on_history_tree_empty_clicked(null, 0)

func _on_history_tree_empty_clicked(_vec2, _idx):
	history_saved_selection = null
	history_tree.deselect_all()
	update_diff()

# Read the selection from the tree, and update the diff visualization accordingly.
func update_diff():
	if !GodotProject.has_project(): return
	var selected_item = history_tree.get_selected()
	var diff;

	if (selected_item == null
			or GodotProject.is_merge_preview_branch_active()
			or GodotProject.is_revert_preview_branch_active()):
		diff = GodotProject.get_default_diff()
		show_diff(diff, false)
	else:
		var hash = get_history_item_hash(selected_item)
		diff = GodotProject.get_diff(hash)
		if (!diff):
			show_invalid_diff()
			return
		show_diff(diff, true)

# Inspect the diff dictionary.
func show_diff(diff, is_change) -> void:
	if !diff:
		inspector.visible = false
		diff_section_header.text = "Changes"
		%ClearDiffButton.visible = false
		return
	last_diff = diff
	%ClearDiffButton.visible = is_change
	inspector.visible = true
	diff_section_header.text = diff.title
	inspector.reset()
	inspector.add_diff(diff.dict)

# Show an invalid diff for a commit with no valid diff (e.g. setup commits)
func show_invalid_diff() -> void:
	inspector.visible = false
	diff_section_header.text = "No diff available for selection"
	%ClearDiffButton.visible = true

func _on_copy_project_id_button_pressed() -> void:
	var project_id = GodotProject.get_project_id()
	if not project_id.is_empty():
		DisplayServer.clipboard_set(project_id)

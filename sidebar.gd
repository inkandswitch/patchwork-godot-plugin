@tool
extends MarginContainer
# This is a Godot 4.x script file, written in GDScript 2.0. Connections are made using the identifier for the callable directly.
# Godot 3.x: something.connect("signal_name", self, "_on_signal_name")
# Godot 4.x: something.connect("signal_name", self._on_signal_name)

var godot_project: GodotProject

@onready var branch_picker: OptionButton = %BranchPicker
@onready var menu_button: MenuButton = %MenuButton
@onready var history_list: ItemList = %HistoryList
@onready var change_count_label: Label = %ChangeCountLabel

var branches = []
var plugin: EditorPlugin
const CREATE_BRANCH_IDX = 1
const MERGE_BRANCH_IDX = 2

func init(plugin: EditorPlugin, godot_project: GodotProject) -> void:
	print("Sidebar initialized!")
	self.godot_project = godot_project
	self.plugin = plugin

func _on_resource_saved(path):
	print("Resource saved: %s" % [path])
func _on_scene_saved(path):
	print("Scene saved: %s" % [path])
	
func _update_ui_on_branches_changed(branches: Array):
	print("Branches changed, updating UI", branches)
	update_ui()

func _update_ui_on_files_changed():
	print("Files changed, updating UI")
	update_ui()

# TODO: It seems that Sidebar is being instantiated by the editor before the plugin does?
func _ready() -> void:
	print("Sidebar ready!")
	branch_picker.item_selected.connect(_on_branch_picker_item_selected)
	update_ui()
	plugin.connect("resource_saved", self._on_resource_saved)
	plugin.connect("scene_saved", self._on_scene_saved)
	godot_project.connect("branches_changed", self._update_ui_on_branches_changed);
	godot_project.connect("files_changed", self._update_ui_on_files_changed);
	var popup = menu_button.get_popup()
	popup.id_pressed.connect(_on_menu_button_id_pressed)

func _on_branch_picker_item_selected(index: int) -> void:
	var selected_branch = branches[index]
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

# This should be called before any patchwork source control action (e.g. checkout, merge, etc.)
func _before_cvs_action(cvs_action: String):
	EditorInterface.save_all_scenes();
	current_cvs_action.append(cvs_action)
	PatchworkEditor.progress_add_task(cvs_action, cvs_action, 10, false)
	plugin.sync_godot_to_patchwork()

func _after_cvs_action():
	if not current_cvs_action.is_empty():
		for i in range(current_cvs_action.size()):
			PatchworkEditor.progress_end_task(current_cvs_action[i])
		current_cvs_action = []

func _merge_branch():
	godot_project.merge_branch(godot_project.get_checked_out_branch_id())
	godot_project.checkout_branch("main");
	print("checked out!")

func merge_branch():
	_before_cvs_action("Merging branch")
	call_deferred("_merge_branch")

func _on_menu_button_id_pressed(id: int) -> void:
	match id:
		CREATE_BRANCH_IDX:
			if PatchworkEditor.unsaved_files_open():
				popup_box(self, $ConfirmationDialog, "You have unsaved files open. Do you want to save them before creating a new branch?", "Unsaved Files", self._on_create_new_branch)
			else:
				_on_create_new_branch()

		MERGE_BRANCH_IDX:
			# check if we're on the main branch or not
			var found = false
			var checked_out_branch = godot_project.get_checked_out_branch_id()
			var selected_id = branch_picker.get_selected_id()
			for i in range(branch_picker.item_count):
				if i == selected_id and branch_picker.get_item_text(i) == "main":
					popup_box(self, $ErrorDialog, "Can't merge the main branch!", "Error")
					return
				elif checked_out_branch == branch_picker.get_item_metadata(i) and branch_picker.get_item_text(i) == "main":
					popup_box(self, $ErrorDialog, "Can't merge the main branch and shouldn't have gotten here!!", "Error")
					return
			var branches: Array = godot_project.get_branches()
			# print("current branches:")
			for branch in branches:
				print("%s: %s" % [branch.id, branch.name])
				if branch.id == checked_out_branch:
					found = true
					# print("Current checked out branch: %s" % [branch.name])
					if branch.name == "main":
						popup_box(self, $ErrorDialog, "Can't merge the main branch and shouldn't have gotten here!!", "Error")
						return
			if PatchworkEditor.unsaved_files_open():
				popup_box(self, $ConfirmationDialog, "You have unsaved files open. Do you want to save them before merging?", "Unsaved Files", self.merge_branch)
			else:
				merge_branch()
			pass

func _checkout_branch(branch_id: String) -> void:
	_before_cvs_action("Checking out branch")
	godot_project.checkout_branch(branch_id)

func checkout_branch(branch_id: String) -> void:
	if PatchworkEditor.unsaved_files_open():
		popup_box(self, $ConfirmationDialog, "You have unsaved files open. Do you want to save them before checking out?", "Unsaved Files", self._checkout_branch.bind(branch_id))
		return
	_checkout_branch(branch_id)

func _on_create_new_branch() -> void:
	var dialog = ConfirmationDialog.new()
	dialog.title = "Create New Branch"
	
	var line_edit = LineEdit.new()
	line_edit.placeholder_text = "Branch name"
	dialog.add_child(line_edit)
	
	# Position line edit in dialog
	line_edit.position = Vector2(8, 8)
	line_edit.size = Vector2(200, 30)
	
	# Make dialog big enough for line edit
	dialog.size = Vector2(220, 100)
	
	dialog.get_ok_button().text = "Create"
	dialog.canceled.connect(func(): dialog.queue_free())
	
	dialog.confirmed.connect(func():
		if line_edit.text.strip_edges() != "":
			_before_cvs_action("Creating new branch")
			var new_branch_name = line_edit.text.strip_edges()
			godot_project.create_branch(new_branch_name)
		
		dialog.queue_free()
	)
	
	
	add_child(dialog)
	dialog.popup_centered()

func update_ui() -> void:
	var checked_out_branch = godot_project.get_checked_out_branch_id()

	if not godot_project:
		# update_ui() called before init
		print("ERROR: update_ui() called before init")
		return
	self.branches = godot_project.get_branches()

	# update branch picker
	branch_picker.clear()
	var checked_out_branch_id = godot_project.get_checked_out_branch_id() if checked_out_branch == null else checked_out_branch
	for i in range(branches.size()):
		var branch = branches[i]
		branch_picker.add_item(branch.name, i)
		branch_picker.set_item_metadata(i, branch.id)
		if branch.id == checked_out_branch_id:
			branch_picker.select(i)

	# update history
	
	var history = godot_project.get_changes()
	history_list.clear()

	change_count_label.text = str(history.size()) + " change" if history.size() == 1 else str(history.size()) + " changes"

	for change in history:
		history_list.add_item(change)

	# update context menu
	var menu_popup = menu_button.get_popup()
	
	menu_popup.clear()

	menu_popup.add_item("Create new branch", CREATE_BRANCH_IDX) # Create new branch menu item
	menu_popup.add_item("Merge branch", MERGE_BRANCH_IDX)

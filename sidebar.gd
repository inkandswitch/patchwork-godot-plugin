@tool
extends MarginContainer

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
	self.godot_project = godot_project
	self.plugin = plugin

func _ready() -> void:
	branch_picker.item_selected.connect(_on_branch_picker_item_selected)
	update_ui()

	godot_project.connect("branches_changed", update_ui);
	godot_project.connect("files_changed", update_ui);

	var popup = menu_button.get_popup()
	popup.id_pressed.connect(_on_menu_button_id_pressed)

func _on_branch_picker_item_selected(index: int) -> void:
	var selected_branch = branches[index]
	godot_project.checkout_branch(selected_branch.id)
	update_ui()


func _on_menu_button_id_pressed(id: int) -> void:
	match id:
		CREATE_BRANCH_IDX:
			_on_create_new_branch()

		MERGE_BRANCH_IDX:
			godot_project.merge_branch(godot_project.get_checked_out_branch_id())
			godot_project.checkout_branch("main");
			pass


func checkout_branch(branch_id: String) -> void:
	EditorInterface.save_all_scenes();
	godot_project.checkout_branch(branch_id)
	update_ui()
	
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
			var new_branch_name = line_edit.text.strip_edges()
			var new_branch_id = godot_project.create_branch(new_branch_name)
			checkout_branch(new_branch_id)
		
		dialog.queue_free()
	)
	
	
	add_child(dialog)
	dialog.popup_centered()


func update_ui() -> void:
	self.branches = godot_project.get_branches()

	# update branch picker
	branch_picker.clear()

	var checked_out_branch_id = godot_project.get_checked_out_branch_id()
	for i in range(branches.size()):
		var branch = branches[i]
		branch_picker.add_item(branch.name, i)

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

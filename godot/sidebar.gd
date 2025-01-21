@tool
extends MarginContainer

var godot_project: GodotProject

@onready var branch_picker: OptionButton = %BranchPicker
@onready var new_branch_button: Button = %NewBranchButton
@onready var change_count_label: Label = %ChangeCountLabel
@onready var reload_button: Button = %ReloadButton

var branches = []
var plugin: EditorPlugin

func init(plugin: EditorPlugin, godot_project: GodotProject) -> void:
  self.godot_project = godot_project
  self.plugin = plugin

func _ready() -> void:
  new_branch_button.pressed.connect(_on_new_branch_button_pressed)
  branch_picker.item_selected.connect(_on_branch_picker_item_selected)
  reload_button.pressed.connect(update_branches)
  update_branches()

func _on_branch_picker_item_selected(index: int) -> void:
  var selected_branch = branches[index]
  godot_project.checkout_branch(selected_branch.id)
  update_branches()

func checkout_branch(branch_id: String) -> void:
  godot_project.checkout_branch(branch_id)
  update_branches()
  plugin.get_editor_interface().save_all_scenes()
  
func _on_new_branch_button_pressed() -> void:
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


func update_branches() -> void:
  self.branches = godot_project.get_branches()

  branch_picker.clear()
  
  var changes_count = godot_project.get_changes().size()
  change_count_label.text = str(changes_count) + " " + ("change" if changes_count == 1 else "changes")

  var checked_out_branch_id = godot_project.get_checked_out_branch_id()
  for i in range(branches.size()):
    var branch = branches[i]
    branch_picker.add_item(branch.name, i)

    if branch.id == checked_out_branch_id:
      branch_picker.select(i)

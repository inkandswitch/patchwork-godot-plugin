@tool
extends MarginContainer

var editor_interface: EditorInterface
var undo_redo_manager: EditorUndoRedoManager

var DEBUG_MODE = false

@onready var simulated_edits_label = %SimulatedEditsLabel
@onready var simulated_edits_panel = %SimulatedEditsPanel
@onready var simulated_edits_checkbox: CheckButton = %SimulatedEditsToggle
@onready var simulated_edits_frequency: Slider = %SimulatedEditsFrequency
@onready var branch_picker: OptionButton = %BranchPicker
@onready var new_branch_button: Button = %NewBranchButton

signal create_new_branch(branch_name: String)
signal checkout_branch(branch_doc_id: String)

var branches = []

func init(editor_plugin: EditorPlugin) -> void:
  self.editor_interface = editor_plugin.get_editor_interface()
  self.undo_redo_manager = editor_plugin.get_undo_redo()

func _ready() -> void:
  if !DEBUG_MODE:
    simulated_edits_label.hide()
    simulated_edits_panel.hide()

  new_branch_button.pressed.connect(_on_new_branch_button_pressed)
  branch_picker.item_selected.connect(_on_branch_picker_item_selected)

func _on_branch_picker_item_selected(index: int) -> void:
  var selected_branch = branches[index]
  checkout_branch.emit(selected_branch.id)


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
      var branch_name = line_edit.text.strip_edges()
      create_new_branch.emit(branch_name)
    dialog.queue_free()
  )
  
  add_child(dialog)
  dialog.popup_centered()


func update_branches(branches) -> void:
  self.branches = branches
  
  branch_picker.clear()
  for i in range(branches.size()):
    var branch = branches[i]
    print("add", branch.name, branch.id)
    branch_picker.add_item(branch.name, i)

var last_update_time: int = 0
func _process(_delta: float) -> void:
  pass
  if !editor_interface:
    return

  var do_simulated_edits = simulated_edits_checkbox.is_pressed()

  var current_time = Time.get_ticks_msec()
  if do_simulated_edits:
    var selected_nodes = editor_interface.get_selection().get_selected_nodes()
    if selected_nodes.size() == 0:
      return
      
    var selected_node = selected_nodes[0]
    
    var frequency = simulated_edits_frequency.max_value - (simulated_edits_frequency.value - simulated_edits_frequency.min_value)
    if (current_time - last_update_time) >= frequency:
      undo_redo_manager.create_action("Rotate node randomly")
      undo_redo_manager.add_do_property(selected_node, "rotation_degrees", randf_range(-180, 180))
      undo_redo_manager.add_undo_property(selected_node, "rotation_degrees", selected_node.rotation_degrees)
      undo_redo_manager.commit_action()

      last_update_time = current_time

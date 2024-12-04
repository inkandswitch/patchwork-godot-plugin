@tool
extends MarginContainer

var editor_interface: EditorInterface
var undo_redo_manager: EditorUndoRedoManager

@onready var simulated_edits_checkbox: CheckButton = %SimulatedEditsToggle
@onready var simulated_edits_frequency: Slider = %SimulatedEditsFrequency

func init(editor_plugin: EditorPlugin) -> void:
  self.editor_interface = editor_plugin.get_editor_interface()
  self.undo_redo_manager = editor_plugin.get_undo_redo()

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

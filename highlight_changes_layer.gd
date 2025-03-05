@tool
class_name HighlightChangesLayer
extends Node2D

var overlay_size: Vector2
var overlay_position: Vector2
var root: Node

func _ready():
	# Ensure we're visible in the editor
	if Engine.is_editor_hint():
		set_process(true)
		set_notify_transform(true)

func _draw():
	# draw overlay to make everything apear grayed out
	draw_rect(Rect2(overlay_position, overlay_size), Color(77.0 / 255.0, 77.0 / 255.0, 77.0 / 255.0, 0.75), true)

	# draw changed shapes
	var coins = root.find_children("Coin")
	for coin in coins:
		draw_rect(_get_node_bounding_box(coin), Color(0.4, 0.96, 0.34, 0.75), true)


static func highlight_changes(root: Node):
	var highlight_changes_layer_container = root.get_node_or_null("PatchworkHighlightChangesLayerContainer")

	if highlight_changes_layer_container == null:
		highlight_changes_layer_container = CanvasLayer.new()
		highlight_changes_layer_container.name = "PatchworkHighlightChangesLayerContainer"
		highlight_changes_layer_container.layer = 1025
		root.add_child(highlight_changes_layer_container)

	var diff_layer = highlight_changes_layer_container.get_node_or_null("PatchworkHighlightChangesLayer")

	var bounding_box = _get_node_bounding_box(root)

	if diff_layer == null:
		diff_layer = HighlightChangesLayer.new()
		diff_layer.name = "PatchworkHighlightChangesLayer"
		diff_layer.root = root
		highlight_changes_layer_container.add_child(diff_layer)

	diff_layer.overlay_position = bounding_box.position - Vector2(bounding_box.size.x, 0)
	diff_layer.overlay_size = bounding_box.size * 3


static func _get_node_bounding_box(node: Node):
	# Initialize with an empty rect
	var bounding_box
	
	# Special handling for collision shapes
	if node is CollisionShape2D:
		var shape = node.shape
		var rect = Rect2()
		
		if shape is CircleShape2D:
			var radius = shape.radius
			rect = Rect2(- radius, - radius, radius * 2, radius * 2)
		elif shape is RectangleShape2D:
			var extents = shape.extents
			rect = Rect2(- extents.x, - extents.y, extents.x * 2, extents.y * 2)
		elif shape is CapsuleShape2D:
			var radius = shape.radius
			var height = shape.height
			rect = Rect2(- radius, - height / 2 - radius, radius * 2, height + radius * 2)
		
		# Convert to global coordinates
		var transform = node.get_global_transform()
		rect = transform * rect
		bounding_box = rect
	# Check if the node has a get_rect method
	elif node.has_method("get_rect"):
		var rect = node.get_rect()
		# Convert to global coordinates
		if node.has_method("get_global_transform"):
			var transform = node.get_global_transform()
			rect = transform * rect
		bounding_box = rect
	
	# Recursively process all children
	for child in node.get_children():
		# Get the bounding box of the child
		var child_rect = _get_node_bounding_box(child)
		
		# Skip empty rects
		if child_rect == null or child_rect.size.x <= 0 or child_rect.size.y <= 0:
			continue
			
		# If this is the first valid rect, use it directly
		if bounding_box == null:
			bounding_box = child_rect
		else:
			bounding_box = bounding_box.merge(child_rect)
		
	return bounding_box
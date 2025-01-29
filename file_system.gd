@tool
class_name FileSystem

var file_contents: Dictionary = {}
var editor_plugin: EditorPlugin
var _ignore_changes = false

signal file_changed(path: String, file_name: String)

func _init(editor_plugin: EditorPlugin):
	self.editor_plugin = editor_plugin

	# load initial state of files
	for path in list_all_files():
		file_contents[path] = get_file(path)

	# listen to file system
	var file_system = editor_plugin.get_editor_interface().get_resource_filesystem()
	file_system.connect("filesystem_changed", _on_filesystem_changed)
	file_system.connect("resources_reload", _on_resources_reloaded)

	# disable granular updates for now

	# listen to changes of scene file
	# editor_plugin.get_undo_redo().connect("version_changed", _on_changed)
	# editor_plugin.get_undo_redo().connect("history_changed", _on_changed)


func ignore_changes(callback: Callable) -> void:
	_ignore_changes = true
	callback.call()
	_ignore_changes = false
	
func stop():
	var file_system = editor_plugin.get_editor_interface().get_resource_filesystem()

	# Cleanup connections when plugin is disabled
	if file_system:
		pass
		#file_system.disconnect("filesystem_changed", _on_filesystem_changed)
		#file_system.disconnect("resources_reload", _on_resources_reloaded)

func trigger_file_changed(file_path: String, content: Variant) -> void:
	print("?? trigger file changed?", file_path)

	var stored_content = file_contents.get(file_path, "")
	if content != stored_content:
		print("!! trigger file changed!")

	file_contents[file_path] = content
	file_changed.emit(file_path, content)

func list_all_files() -> Array[String]:
	var files: Array[String] = []
	var dir = DirAccess.open("res://")
	if dir:
		_scan_directory_for_files(dir, "res://", files)
	return files

func _scan_directory_for_files(dir: DirAccess, current_path: String, files: Array[String]) -> void:
	dir.list_dir_begin()
	var file_name = dir.get_next()
	
	while file_name != "":
		if file_name == "." or file_name == "..":
			file_name = dir.get_next()
			continue
			
		var full_path = current_path.path_join(file_name)
		
		if dir.current_is_dir():
			var sub_dir = DirAccess.open(full_path)
			if sub_dir:
				_scan_directory_for_files(sub_dir, full_path, files)
		else:
			files.append(full_path)
			
		file_name = dir.get_next()

func is_binary(file) -> bool:
	# Read first chunk to detect if binary
	var test_bytes = file.get_buffer(min(1024, file.get_length()))
	
	# Reset file position
	file.seek(0)

	# Check for null bytes and high ratio of non-printable chars
	var non_printable = 0
	
	for i in range(test_bytes.size()):
		if test_bytes[i] == 0 or (test_bytes[i] < 32 and test_bytes[i] != 10 and test_bytes[i] != 13 and test_bytes[i] != 9):
			non_printable += 1
	return (non_printable / float(test_bytes.size())) > 0.3

func get_file(path: String):
	var file = FileAccess.open(path, FileAccess.READ)
	var content
	if file:
		if is_binary(file):
			# Handle binary files by reading raw bytes
			return file.get_buffer(file.get_length())

		# Handle text files
		return file.get_as_text()

	return null

func delete_file(path: String) -> void:
	if FileAccess.file_exists(path):
		DirAccess.remove_absolute(path)
		file_contents.erase(path)


func save_file(path: String, content: String) -> void:
	# Create directory structure if it doesn't exist
	var dir_path = path.get_base_dir()
	if !DirAccess.dir_exists_absolute(dir_path):
		DirAccess.make_dir_recursive_absolute(dir_path)
		
	var file = FileAccess.open(path, FileAccess.WRITE)
	if file:
		file.store_string(content)
		file_contents[path] = content


## FILE SYSTEM CHANGED

func _on_filesystem_changed():
	_scan_for_changes()

func _on_resources_reloaded(resources: Array):
	for path in resources:
			_check_file_changes(path)

func _scan_for_changes():
	var dir = DirAccess.open("res://")
	if dir:
			_scan_directory(dir, "res://")

func _scan_directory(dir: DirAccess, current_path: String):
	# Recursively scan directories for files
	dir.list_dir_begin()
	var file_name = dir.get_next()
	
	while file_name != "":
			if file_name == "." or file_name == "..":
					file_name = dir.get_next()
					continue
					
			var full_path = current_path.path_join(file_name)
			
			if dir.current_is_dir():
					var sub_dir = DirAccess.open(full_path)
					if sub_dir:
							_scan_directory(sub_dir, full_path)
			else:
					_check_file_changes(full_path)
					
			file_name = dir.get_next()

func _check_file_changes(file_path: String):

	print("check file ", file_path);

	var content = get_file(file_path)
	if not content:
			print("error no file found", file_path);
			return

	trigger_file_changed(file_path, content)


## SCENE CHANGED

# todo: figure out how to do this without creating a temp file
# todo: figure out how to make ids stable
func _on_changed():
	if _ignore_changes:
		return

	var root = editor_plugin.get_editor_interface().get_edited_scene_root()
	if root:
		var packed_scene = PackedScene.new()
		packed_scene.pack(root)
		
		var temp_path = "user://scene.tscn"
		
		# Save to temp file
		var error = ResourceSaver.save(packed_scene, temp_path)
		if error != OK:
			print("Error saving scene: ", error)
			return
			
		# Read the file contents
		var file = FileAccess.open(temp_path, FileAccess.READ)
		if file:
			var content = file.get_as_text()
			trigger_file_changed(root.scene_file_path, content)
			file.close()


		# Delete the temp file
		DirAccess.remove_absolute(temp_path)

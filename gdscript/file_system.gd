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
	var file_system: EditorFileSystem = editor_plugin.get_editor_interface().get_resource_filesystem()
	
	connect_to_file_system()

func connect_to_file_system():
	var file_system = editor_plugin.get_editor_interface().get_resource_filesystem()
	file_system.connect("filesystem_changed", _on_filesystem_changed)
	file_system.connect("resources_reload", _on_resources_reloaded)

func disconnect_from_file_system():
	var file_system = editor_plugin.get_editor_interface().get_resource_filesystem()
	file_system.disconnect("filesystem_changed", _on_filesystem_changed)
	file_system.disconnect("resources_reload", _on_resources_reloaded)


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

func check_has_file_changed(file_path: String, content: Variant) -> void:
	if not editor_plugin._is_relevant_file(file_path):
		return
	var stored_content = file_contents.get(file_path)

	# Handle case where file is new
	if stored_content == null:
		file_contents[file_path] = content
		file_changed.emit(file_path, content)
		return

	# Compare contents based on type
	var has_changed = _is_content_equal(content, stored_content)

	if has_changed:
		file_contents[file_path] = content
		file_changed.emit(file_path, content)

func _is_content_equal(content_a: Variant, content_b: Variant) -> bool:
	if content_a is String and content_b is String:
		return content_a != content_b
	elif content_a is PackedByteArray and content_b is PackedByteArray:
		return content_a != content_b
	else:
		# Type mismatch - file changed from text to binary or vice versa
		return true

func list_all_files() -> Array[String]:
	var files: Array[String] = []
	var dir = DirAccess.open("res://")
	dir.include_hidden = true
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
		if not editor_plugin._is_relevant_file(full_path):
			file_name = dir.get_next()
			continue
		if dir.current_is_dir():
			var sub_dir = DirAccess.open(full_path)
			if sub_dir:
				_scan_directory_for_files(sub_dir, full_path, files)
		else:
			files.append(full_path)

		file_name = dir.get_next()


var _control_chars = PackedByteArray([10, 13, 9, 12, 8])
var _printable_ascii = _control_chars + PackedByteArray(range(32, 127))
var _printable_high_ascii = PackedByteArray(range(127, 256))

func is_binary_string(bytes_to_check: PackedByteArray) -> bool:
	if bytes_to_check.size() == 0:
		return false

	var low_chars = PackedByteArray()
	for byte in bytes_to_check:
		if not _printable_ascii.has(byte):
			low_chars.append(byte)
	var nontext_ratio1 = float(low_chars.size()) / float(bytes_to_check.size())
	print("nontext_ratio1: ", nontext_ratio1)
	var high_chars = PackedByteArray()
	for byte in bytes_to_check:
		if not _printable_high_ascii.has(byte):
			high_chars.append(byte)
	var nontext_ratio2 = float(high_chars.size()) / float(bytes_to_check.size())
	print("nontext_ratio2: ", nontext_ratio2)
	if nontext_ratio1 > 0.90 and nontext_ratio2 > 0.90:
		return true

	var is_likely_binary = (nontext_ratio1 > 0.3 and nontext_ratio2 < 0.05) or (nontext_ratio1 > 0.8 and nontext_ratio2 > 0.8)

	# UTF-8 is the only encoding that Godot supports, sooooo...
	var decodable_as_unicode = PatchworkEditor.detect_utf8(bytes_to_check)
	if is_likely_binary:
		if decodable_as_unicode:
			return false
		else:
			return true
	else:
		if decodable_as_unicode:
			return false
		else:
			if 0 in bytes_to_check or 255 in bytes_to_check:
				# Check for NULL bytes last
				return true
		return false

func is_binary(file) -> bool:
	# Read first chunk to detect if binary
	var test_bytes = file.get_buffer(min(8000, file.get_length()))

	# Reset file position
	file.seek(0)
	var ret: bool
	# This is what git does to detect binary files; it just checks the first 8000 bytes for null bytes
	if 0 in test_bytes:
		ret = true
	# if it's valid unicode, it's not binary
	elif PatchworkEditor.detect_utf8(test_bytes):
		ret = false
	# otherwise, we do a more thorough check
	else:
		#print_debug("More thorough check on ", file.get_path())
		ret = is_binary_string(test_bytes)
	#print_debug("%s is binary: %s" % [file.get_path(), str(ret)])
	return ret

func get_file(path: String) -> Variant:
	var file = FileAccess.open(path, FileAccess.READ)
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


func save_file(path: String, content: Variant) -> void:
	# Create directory structure if it doesn't exist
	var dir_path = path.get_base_dir()
	if !DirAccess.dir_exists_absolute(dir_path):
		DirAccess.make_dir_recursive_absolute(dir_path)
	var file = FileAccess.open(path, FileAccess.WRITE)
	if file:
		if content is String:
			file.store_string(content)
		else:
			file.store_buffer(content)
		file_contents[path] = content
	else:
		print("ERROR: save_file(): Failed to open file for writing: ", path)


## FILE SYSTEM CHANGED

func _on_filesystem_changed():
	# wait until the editor finishes importing resources
	if PatchworkEditor.is_editor_importing():
		return
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
			if not editor_plugin._is_relevant_file(full_path):
				file_name = dir.get_next()
				continue

			if dir.current_is_dir():
					var sub_dir = DirAccess.open(full_path)
					if sub_dir:
							_scan_directory(sub_dir, full_path)
			else:
					_check_file_changes(full_path)

			file_name = dir.get_next()

func _check_file_changes(file_path: String):
	if not editor_plugin._is_relevant_file(file_path):
		return
	var content = get_file(file_path)
	if content == null:
		# Handle deleted files
		if file_contents.has(file_path):
			file_contents.erase(file_path)
			file_changed.emit(file_path, null)
		return

	check_has_file_changed(file_path, content)

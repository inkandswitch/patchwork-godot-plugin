/*************************************************************************/
/*  register_types.cpp                                                   */
/*************************************************************************/

#include "register_types.h"
#include "editor/editor_node.h"
#include "editor/patchwork_editor.h"

void patchwork_editor_init_callback() {
	EditorNode *editor = EditorNode::get_singleton();
	editor->add_child(memnew(PatchworkEditor(editor)));
}

void initialize_patchwork_editor_module(ModuleInitializationLevel p_level) {
	if (p_level == MODULE_INITIALIZATION_LEVEL_EDITOR) {
		EditorNode::add_init_callback(&patchwork_editor_init_callback);
	}
	if (p_level == MODULE_INITIALIZATION_LEVEL_SCENE) {
		ClassDB::register_class<PatchworkEditor>();
		// register the editor inspector section
	}
}

void uninitialize_patchwork_editor_module(ModuleInitializationLevel p_level) {
}

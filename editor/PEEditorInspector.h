
#pragma once
#include <editor/editor_inspector.h>

class PEEditorInspector : public EditorInspector {
	GDCLASS(PEEditorInspector, EditorInspector);

protected:
	static void _bind_methods();

public:
	void set_edit_object(Object *p_object) {
		edit(p_object);
	}
	Object *get_edit_object() {
		return get_edited_object();
	}
};

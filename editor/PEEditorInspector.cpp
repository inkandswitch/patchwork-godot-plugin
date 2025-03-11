
#include "PEEditorInspector.h"
void PEEditorInspector::_bind_methods() {
	ClassDB::bind_method(D_METHOD("set_edit_object", "object"), &PEEditorInspector::set_edit_object);
	ClassDB::bind_method(D_METHOD("get_edit_object"), &PEEditorInspector::get_edit_object);
	// edit_object prop
	ADD_PROPERTY(PropertyInfo(Variant::OBJECT, "edit_object", PROPERTY_HINT_RESOURCE_TYPE, "Object"), "set_edit_object", "get_edit_object");
}

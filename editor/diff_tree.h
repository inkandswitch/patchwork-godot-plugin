
#pragma once
#include "scene/gui/control.h"

class DiffTree : public Control {
	GDCLASS(DiffTree, Control);

protected:
	void _notification(int p_what);
	static void _bind_methods();

public:
	DiffTree();
	~DiffTree();
};

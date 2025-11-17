#include "core/version_generated.gen.h"
#if GODOT_VERSION_MAJOR == 4 && GODOT_VERSION_MINOR < 5
#include "editor/editor_vcs_interface.h"
#else
#include "editor/version_control/editor_vcs_interface.h"
#endif
#include "scene/gui/rich_text_label.h"
class TextDiffer : public Object {
	GDCLASS(TextDiffer, Object);

protected:
	static void _bind_methods();

public:
	static RichTextLabel *get_text_diff(const Dictionary &diff, bool p_split_view = false);
	static void _display_diff_split_view(RichTextLabel *diff, List<EditorVCSInterface::DiffLine> &p_diff_content);
	static void _display_diff_unified_view(RichTextLabel *diff, List<EditorVCSInterface::DiffLine> &p_diff_content);
};

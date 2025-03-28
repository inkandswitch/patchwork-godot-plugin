#include "editor/editor_vcs_interface.h"
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

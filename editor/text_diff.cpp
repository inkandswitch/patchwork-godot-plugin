#include "text_diff.h"
#include "editor/editor_node.h"
#include "editor/editor_string_names.h"
#if GODOT_VERSION_MAJOR == 4 && GODOT_VERSION_MINOR < 5
#include "editor/editor_vcs_interface.h"
#include "editor/plugins/version_control_editor_plugin.h"
#else
#include "editor/version_control/editor_vcs_interface.h"
#include "editor/version_control/version_control_editor_plugin.h"
#endif
#include "scene/gui/rich_text_label.h"
using DiffViewType = VersionControlEditorPlugin::DiffViewType;

EditorVCSInterface::DiffLine _convert_diff_line(const Dictionary &p_diff_line) {
	EditorVCSInterface::DiffLine d;
	d.new_line_no = p_diff_line["new_line_no"];
	d.old_line_no = p_diff_line["old_line_no"];
	d.content = p_diff_line["content"];
	d.status = p_diff_line["status"];
	return d;
}

EditorVCSInterface::DiffHunk _convert_diff_hunk(const Dictionary &p_diff_hunk) {
	EditorVCSInterface::DiffHunk dh;
	dh.new_lines = p_diff_hunk["new_lines"];
	dh.old_lines = p_diff_hunk["old_lines"];
	dh.new_start = p_diff_hunk["new_start"];
	dh.old_start = p_diff_hunk["old_start"];
	TypedArray<Dictionary> diff_lines = p_diff_hunk["diff_lines"];
	for (int i = 0; i < diff_lines.size(); i++) {
		EditorVCSInterface::DiffLine dl = _convert_diff_line(diff_lines[i]);
		dh.diff_lines.push_back(dl);
	}
	return dh;
}

EditorVCSInterface::DiffFile _convert_diff_file(const Dictionary &p_diff_file) {
	EditorVCSInterface::DiffFile df;
	df.new_file = p_diff_file["new_file"];
	df.old_file = p_diff_file["old_file"];
	TypedArray<Dictionary> diff_hunks = p_diff_file["diff_hunks"];
	for (int i = 0; i < diff_hunks.size(); i++) {
		EditorVCSInterface::DiffHunk dh = _convert_diff_hunk(diff_hunks[i]);
		df.diff_hunks.push_back(dh);
	}
	return df;
}

RichTextLabel *TextDiffer::get_text_diff(const Dictionary &_diff, bool p_split_view) {
	EditorVCSInterface::DiffFile diff_file = _convert_diff_file(_diff);
	auto diff = memnew(RichTextLabel);
	diff->push_font(EditorNode::get_singleton()->get_editor_theme()->get_font(SNAME("doc_bold"), EditorStringName(EditorFonts)));
	diff->push_color(EditorNode::get_singleton()->get_editor_theme()->get_color(SNAME("accent_color"), EditorStringName(Editor)));
	diff->add_text(TTR("File:") + " " + diff_file.new_file);
	diff->pop();
	diff->pop();

	diff->push_font(EditorNode::get_singleton()->get_editor_theme()->get_font(SNAME("status_source"), EditorStringName(EditorFonts)));
	for (EditorVCSInterface::DiffHunk hunk : diff_file.diff_hunks) {
		String old_start = String::num_int64(hunk.old_start);
		String new_start = String::num_int64(hunk.new_start);
		String old_lines = String::num_int64(hunk.old_lines);
		String new_lines = String::num_int64(hunk.new_lines);

		diff->add_newline();
		diff->append_text("[center]@@ " + old_start + "," + old_lines + " " + new_start + "," + new_lines + " @@[/center]");
		diff->add_newline();

		if (p_split_view) {
			_display_diff_split_view(diff, hunk.diff_lines);
		} else {
			_display_diff_unified_view(diff, hunk.diff_lines);
		}
		diff->add_newline();
	}
	diff->pop();

	diff->add_newline();
	return diff;
}

void TextDiffer::_display_diff_split_view(RichTextLabel *diff, List<EditorVCSInterface::DiffLine> &p_diff_content) {
	LocalVector<EditorVCSInterface::DiffLine> parsed_diff;

	for (EditorVCSInterface::DiffLine diff_line : p_diff_content) {
		String line = diff_line.content.strip_edges(false, true);

		if (diff_line.new_line_no >= 0 && diff_line.old_line_no >= 0) {
			diff_line.new_text = line;
			diff_line.old_text = line;
			parsed_diff.push_back(diff_line);
		} else if (diff_line.new_line_no == -1) {
			diff_line.new_text = "";
			diff_line.old_text = line;
			parsed_diff.push_back(diff_line);
		} else if (diff_line.old_line_no == -1) {
			int32_t j = parsed_diff.size() - 1;
			while (j >= 0 && parsed_diff[j].new_line_no == -1) {
				j--;
			}

			if (j == (int32_t)parsed_diff.size() - 1) {
				// no lines are modified
				diff_line.new_text = line;
				diff_line.old_text = "";
				parsed_diff.push_back(diff_line);
			} else {
				// lines are modified
				EditorVCSInterface::DiffLine modified_line = parsed_diff[j + 1];
				modified_line.new_text = line;
				modified_line.new_line_no = diff_line.new_line_no;
				parsed_diff[j + 1] = modified_line;
			}
		}
	}

	diff->push_table(6);
	/*
		[cell]Old Line No[/cell]
		[cell]prefix[/cell]
		[cell]Old Code[/cell]

		[cell]New Line No[/cell]
		[cell]prefix[/cell]
		[cell]New Line[/cell]
	*/

	diff->set_table_column_expand(2, true);
	diff->set_table_column_expand(5, true);

	for (uint32_t i = 0; i < parsed_diff.size(); i++) {
		EditorVCSInterface::DiffLine diff_line = parsed_diff[i];

		bool has_change = diff_line.status != " ";
		static const Color red = EditorNode::get_singleton()->get_editor_theme()->get_color(SNAME("error_color"), EditorStringName(Editor));
		static const Color green = EditorNode::get_singleton()->get_editor_theme()->get_color(SNAME("success_color"), EditorStringName(Editor));
		static const Color white = EditorNode::get_singleton()->get_editor_theme()->get_color(SceneStringName(font_color), SNAME("Label")) * Color(1, 1, 1, 0.6);

		if (diff_line.old_line_no >= 0) {
			diff->push_cell();
			diff->push_color(has_change ? red : white);
			diff->add_text(String::num_int64(diff_line.old_line_no));
			diff->pop();
			diff->pop();

			diff->push_cell();
			diff->push_color(has_change ? red : white);
			diff->add_text(has_change ? "-|" : " |");
			diff->pop();
			diff->pop();

			diff->push_cell();
			diff->push_color(has_change ? red : white);
			diff->add_text(diff_line.old_text);
			diff->pop();
			diff->pop();

		} else {
			diff->push_cell();
			diff->pop();

			diff->push_cell();
			diff->pop();

			diff->push_cell();
			diff->pop();
		}

		if (diff_line.new_line_no >= 0) {
			diff->push_cell();
			diff->push_color(has_change ? green : white);
			diff->add_text(String::num_int64(diff_line.new_line_no));
			diff->pop();
			diff->pop();

			diff->push_cell();
			diff->push_color(has_change ? green : white);
			diff->add_text(has_change ? "+|" : " |");
			diff->pop();
			diff->pop();

			diff->push_cell();
			diff->push_color(has_change ? green : white);
			diff->add_text(diff_line.new_text);
			diff->pop();
			diff->pop();
		} else {
			diff->push_cell();
			diff->pop();

			diff->push_cell();
			diff->pop();

			diff->push_cell();
			diff->pop();
		}
	}
	diff->pop();
}

void TextDiffer::_display_diff_unified_view(RichTextLabel *diff, List<EditorVCSInterface::DiffLine> &p_diff_content) {
	diff->push_table(4);
	diff->set_table_column_expand(3, true);

	/*
		[cell]Old Line No[/cell]
		[cell]New Line No[/cell]
		[cell]status[/cell]
		[cell]code[/cell]
	*/
	for (const EditorVCSInterface::DiffLine &diff_line : p_diff_content) {
		String line = diff_line.content.strip_edges(false, true);

		Color color;
		if (diff_line.status == "+") {
			color = EditorNode::get_singleton()->get_editor_theme()->get_color(SNAME("success_color"), EditorStringName(Editor));
		} else if (diff_line.status == "-") {
			color = EditorNode::get_singleton()->get_editor_theme()->get_color(SNAME("error_color"), EditorStringName(Editor));
		} else {
			color = EditorNode::get_singleton()->get_editor_theme()->get_color(SceneStringName(font_color), SNAME("Label"));
			color *= Color(1, 1, 1, 0.6);
		}
		auto diff_old_line_no = diff_line.old_line_no >= 0 ? String::num_int64(diff_line.old_line_no) : "";
		auto diff_new_line_no = diff_line.new_line_no >= 0 ? String::num_int64(diff_line.new_line_no) : "";
		if (diff_line.old_line_no >= 0 && diff_line.new_line_no >= 0) {
			diff_old_line_no += "|";
		}
		diff->push_cell();
		diff->push_color(color);
		diff->push_indent(1);
		diff->add_text(diff_old_line_no);
		diff->pop();
		diff->pop();
		diff->pop();

		diff->push_cell();
		diff->push_color(color);
		diff->push_indent(1);
		diff->add_text(diff_new_line_no);
		diff->pop();
		diff->pop();
		diff->pop();

		diff->push_cell();
		diff->push_color(color);
		diff->add_text(diff_line.status != "" ? diff_line.status + "|" : " |");
		diff->pop();
		diff->pop();

		diff->push_cell();
		diff->push_color(color);
		diff->add_text(line);
		diff->pop();
		diff->pop();
	}

	diff->pop();
}

void TextDiffer::_bind_methods() {
	ClassDB::bind_static_method(get_class_static(), D_METHOD("get_text_diff", "diff", "split_view"), &TextDiffer::get_text_diff, DEFVAL(false));
}

/**************************************************************************/
/*  missing_resource.h                                                    */
/**************************************************************************/
/*                         This file is part of:                          */
/*                             GODOT ENGINE                               */
/*                        https://godotengine.org                         */
/**************************************************************************/
/* Copyright (c) 2014-present Godot Engine contributors (see AUTHORS.md). */
/* Copyright (c) 2007-2014 Juan Linietsky, Ariel Manzur.                  */
/*                                                                        */
/* Permission is hereby granted, free of charge, to any person obtaining  */
/* a copy of this software and associated documentation files (the        */
/* "Software"), to deal in the Software without restriction, including    */
/* without limitation the rights to use, copy, modify, merge, publish,    */
/* distribute, sublicense, and/or sell copies of the Software, and to     */
/* permit persons to whom the Software is furnished to do so, subject to  */
/* the following conditions:                                              */
/*                                                                        */
/* The above copyright notice and this permission notice shall be         */
/* included in all copies or substantial portions of the Software.        */
/*                                                                        */
/* THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,        */
/* EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF     */
/* MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. */
/* IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY   */
/* CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,   */
/* TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE      */
/* SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.                 */
/**************************************************************************/

#ifndef MISSING_RESOURCE_H
#define MISSING_RESOURCE_H

#include "core/io/resource.h"

#define META_PROPERTY_MISSING_RESOURCES "metadata/_missing_resources"
#define META_MISSING_RESOURCES "_missing_resources"

class FakeInspectorResource : public Resource {
	GDCLASS(FakeInspectorResource, Resource)
	HashMap<String, Dictionary> file_diff_map;

	String original_class;
	bool recording_properties = false;

protected:
	bool _set(const StringName &p_name, const Variant &p_value);
	bool get_from_props_dict(const Dictionary &props, const String &p_name, Variant &r_ret) const;
	Dictionary get_prop_dict_for_getset(String fq_prop_name, String &real_prop_name) const;
	bool _get(const StringName &p_name, Variant &r_ret) const;
	void add_props_to_list(const String &name, const Dictionary &props, List<PropertyInfo> *p_list, const String &node_name = "") const;
	bool add_property_list_from_diff(String path, Dictionary diff, List<PropertyInfo> *p_list) const;
	void _get_property_list(List<PropertyInfo> *p_list) const;

	static void _bind_methods();

public:
	void set_original_class(const String &p_class);
	String get_original_class() const;
	void add_file_diff(const String &file, const Dictionary &props);
	void add_diff(const Dictionary &props);
	void set_recording_properties(bool p_enable);
	bool is_recording_properties() const;

	FakeInspectorResource();
};

#endif // MISSING_RESOURCE_H

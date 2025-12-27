/*************************************************************************/
/*  register_types.h                                                     */
/*************************************************************************/

#ifndef PATCHWORK_EDITOR_REGISTER_TYPES_H
#define PATCHWORK_EDITOR_REGISTER_TYPES_H

#include "modules/register_module_types.h"

void initialize_patchwork_editor_module(ModuleInitializationLevel p_level);
void uninitialize_patchwork_editor_module(ModuleInitializationLevel p_level);
void init_ver_regex();
void free_ver_regex();
#endif // PATCHWORK_EDITOR_REGISTER_TYPES_H


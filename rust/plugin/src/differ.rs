
// #[derive(GodotConvert, Var, Export)]
// #[godot(via = GString)]
// enum ChangeType {
// 	Added,
// 	Removed,
// 	Modified,
//     Unchanged
// }

// #[derive(GodotConvert, Var, Export)]
// #[godot(via = GString)]
// enum PropertyChangeType {
//     None,
//     VariantChanged,
//     SubResourceChanged,
//     ExternalResourceChanged
// }

// #[derive(GodotConvert, Var, Export)]
// #[godot(via = GString)]
// enum ObjectChangeType {
//     PropertyChange,
//     NameChange,
//     PathChange,
//     TypeChange
// }


// #[derive(GodotClass)]
// #[class(no_init, base=RefCounted)]
// pub struct FileDiff {
//     base: Base<RefCounted>,
//     path: String,
//     change_type: ChangeType,
//     old_content: Variant,
//     new_content: Variant,
//     old_import_info: Option<Variant>,
//     new_import_info: Option<Variant>,
//     old_resource: Option<Gd<Resource>>,
//     new_resource: Option<Gd<Resource>>,
//     scene_changes: Option<Gd<SceneDiff>>,
// }

// #[derive(GodotClass)]
// #[class(no_init, base=RefCounted)]
// pub struct ObjectChange {
//     base: Base<RefCounted>,
//     name: String,
//     change_type: ObjectChangeType,
//     property_change_type: PropertyChangeType,
//     old_value: Variant,
//     new_value: Variant,
// }

// #[derive(GodotClass)]
// #[class(no_init, base=RefCounted)]
// pub struct ObjectDiff{
//     base: Base<RefCounted>,
//     changes: Array<Gd<ObjectChange>>,
// }

// #[derive(GodotClass)]
// #[class(no_init, base=RefCounted)]
// pub struct SceneDiff {
//     base: Base<RefCounted>,
//     object_changes: Array<Gd<ObjectChange>>,
// }


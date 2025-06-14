use std::collections::HashMap;
use std::collections::HashSet;

use automerge::Automerge;
// Import the modules from the library
use patchwork_rust_core::godot_parser;
use patchwork_rust_core::godot_parser::GodotConnection;
use patchwork_rust_core::godot_parser::GodotScene;
// use pretty_assertions::{assert_eq, assert_ne};

fn get_test_scene_source() -> String {
    r#"[gd_scene load_steps=6 format=3 uid="uid://jnrusvm3gric"]

[ext_resource type="Texture2D" uid="uid://dw612tw7iymyb" path="res://assets/background-layer-1.png" id="1_0qn5k"]
[ext_resource type="Script" path="res://scripts/background.gd" id="1_1jh5j"]
[ext_resource type="Texture2D" uid="uid://dne1wh5fsffy" path="res://assets/background-layer-2.png" id="2_mk66l"]

[sub_resource type="Gradient" id="Gradient_80myt"]
colors = PackedColorArray(0.98, 0.98, 0.98, 1, 0.81, 0.81, 0.81, 1)
offsets = PackedFloat32Array(0.0788732, 1)

[sub_resource type="GradientTexture2D" id="GradientTexture2D_ljotv"]
fill_to = Vector2(0, 1)
gradient = SubResource("Gradient_80myt")
height = 1080
width = 5115

[node name="Background" type="ParallaxBackground"]
follow_viewport_enabled = true
script = ExtResource("1_1jh5j")
scroll_ignore_camera_zoom = true
metadata/patchwork_id = "1122ae43c1054005997967892c521ea9"

[node name="ParallaxLayer" type="ParallaxLayer" parent="."]
motion_mirroring = Vector2(1600, 0)
motion_scale = Vector2(0, 0)
unique_name_in_owner = true
metadata/patchwork_id = "ae876d398eb24a959b9ff1b00d983948"

[node name="ColorRect" type="ColorRect" parent="ParallaxLayer"]
offset_bottom = 98.0
offset_right = 5129.0
offset_top = -2352.0
anchor_bottom = 1.0
anchor_right = 1.0
anchors_preset = 15
color = Color(0.980392, 0.980392, 0.980392, 1)
grow_horizontal = 2
grow_vertical = 2
metadata/patchwork_id = "5b9416e8d96042b6a509f7da3263f687"

[node name="NestedColorRect" type="ColorRect" parent="ParallaxLayer/ColorRect"]
anchor_bottom = 1.0
anchor_right = 1.0
anchors_preset = 15
color = Color(0.980392, 0.980392, 0.980392, 1)
grow_horizontal = 2
grow_vertical = 2
offset_bottom = 98.0
offset_right = 5129.0
offset_top = -2352.0
metadata/patchwork_id = "9a7c3e5b8f2d1a6c4b8e9d7f5a3c1e8b"

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer"]
centered = false
texture = SubResource("GradientTexture2D_ljotv")
metadata/patchwork_id = "50a6b8d7ce2c469098b3416372f9b1b8"

"#.to_string()
}

#[test]
fn test_parse_and_serialize() {
    let source = get_test_scene_source();

    // Parse the scene
    let scene = godot_parser::parse_scene(&source).unwrap();

    // Serialize the scene back to string
    let reserialized = scene.serialize();

    // Verify that the serialized output matches the original input
    assert_eq!(
        source, reserialized,
        "Serialized output should match original input"
    );
}

fn get_test_scene_source_with_connections_source() -> String {
    r#"[gd_scene load_steps=6 format=3 uid="uid://jnrusvm3gric"]

[node name="Root" type="Node2D"]

[node name="GameManager" type="Node2D" parent="."]

[node name="UI" type="Node2D" parent="."]

[node name="Button" type="Button" parent="UI"]

[connection signal="button_pressed" from="UI/Button" to="GameManager" method="_on_button_pressed" flags=3 unbinds=1 binds=["extra_param", 42, true]]"#.to_string()
}

#[test]
fn test_parse_scene_with_connections() {
    let source = get_test_scene_source_with_connections_source();
    let scene = godot_parser::parse_scene(&source).unwrap();

    let game_manager_node = scene
        .nodes
        .values()
        .find(|node| node.name == "GameManager")
        .unwrap();
    let button_node = scene
        .nodes
        .values()
        .find(|node| node.name == "Button")
        .unwrap();

    let connections = scene.connections.values().collect::<Vec<_>>();
    let connection = connections[0];

    assert_eq!(connections.len(), 1);
    assert_eq!(connection.signal, "button_pressed");
    assert_eq!(connection.from_node_id, button_node.id);
    assert_eq!(connection.to_node_id, game_manager_node.id);
    assert_eq!(connection.method, "_on_button_pressed");
    assert_eq!(connection.flags, Some(3));
    assert_eq!(connection.unbinds, Some(1));
    assert_eq!(
        connection.binds,
        Some("[\"extra_param\", 42, true]".to_string())
    );
}

#[test]
fn test_parse_scene_with_duplicate_ids() {
    let source = r#"[gd_scene load_steps=6 format=3 uid="uid://jnrusvm3gric"]
[node name="Root" type="Node2D"]
metadata/patchwork_id = "1122ae43c1054005997967892c521ea9"

[node name="ColorRect1" type="ColorRect" parent="."]
color = Color(0.980392, 0.980392, 0.980392, 1)
metadata/patchwork_id = "5b9416e8d96042b6a509f7da3263f687"

[node name="ColorRect2" type="ColorRect" parent="."]
color = Color(0.980392, 0.980392, 0.980392, 1)
metadata/patchwork_id = "5b9416e8d96042b6a509f7da3263f687"
"#
    .to_string();

    let scene = godot_parser::parse_scene(&source).unwrap();

    let root_node = scene.nodes.get(scene.root_node_id.as_ref().unwrap()).unwrap();

    assert_eq!(root_node.child_node_ids.len(), 2);

    println!("root_node: {:?}", root_node.child_node_ids);

    // first node has original id
    assert_eq!(
        root_node.child_node_ids[0],
        "5b9416e8d96042b6a509f7da3263f687"
    );

    // second node gets assigned a new id
    assert_ne!(
        root_node.child_node_ids[1],
        "5b9416e8d96042b6a509f7da3263f687"
    );

    assert_eq!(
        scene
            .nodes
            .get(root_node.child_node_ids[1].as_str())
            .unwrap()
            .id,
        root_node.child_node_ids[1],
    );
}

#[test]
fn test_resconcile_and_hydrate() {
    let example_scene = godot_parser::GodotScene {
        format: 3,
        load_steps: 0,
        uid: "uid://b8vp42c3k4q7v".to_string(),
        script_class: None,
        resource_type: "packed_scene".to_string(),
        editable_instances: HashSet::new(),
        main_resource: None,
        nodes: HashMap::from([
            (
                "node1".to_string(),
                godot_parser::GodotNode {
                    name: "Root".to_string(),
                    parent_id: None,
                    properties: HashMap::new(),
                    id: "node1".to_string(),
                    type_or_instance: godot_parser::TypeOrInstance::Type("Node2D".to_string()),
                    owner: None,
                    index: None,
                    groups: None,
                    child_node_ids: vec!["node2".to_string(), "node3".to_string()],
                    node_paths: None,
                },
            ),
            (
                "node2".to_string(),
                godot_parser::GodotNode {
                    name: "Sprite".to_string(),
                    parent_id: Some("node1".to_string()),
                    properties: HashMap::from([(
                        "position".to_string(),
                        godot_parser::OrderedProperty::new("Vector2(100.0, 100.0)".to_string(), 0),
                    )]),
                    id: "node2".to_string(),
                    type_or_instance: godot_parser::TypeOrInstance::Type("Sprite2D".to_string()),
                    owner: None,
                    index: None,
                    groups: None,
                    child_node_ids: vec![],
                    node_paths: None,
                },
            ),
            (
                "node3".to_string(),
                godot_parser::GodotNode {
                    name: "Label".to_string(),
                    parent_id: Some("node1".to_string()),
                    properties: HashMap::from([
                        (
                            "offset_right".to_string(),
                            godot_parser::OrderedProperty::new("40.0".to_string(), 0),
                        ),
                        (
                            "offset_bottom".to_string(),
                            godot_parser::OrderedProperty::new("23.0".to_string(), 1),
                        ),
                        (
                            "text".to_string(),
                            godot_parser::OrderedProperty::new("\"Hello World\"".to_string(), 2),
                        ),
                    ]),
                    id: "node3".to_string(),
                    type_or_instance: godot_parser::TypeOrInstance::Type("Label".to_string()),
                    owner: None,
                    index: None,
                    groups: None,
                    child_node_ids: vec![],
                    node_paths: None,
                },
            ),
        ]),
        root_node_id: Some("node1".to_string()),
        ext_resources: HashMap::from([
            (
                "1_0qn5k".to_string(),
                godot_parser::ExternalResourceNode {
                    idx: 0,
                    id: "1_0qn5k".to_string(),
                    path: "res://assets/background-layer-1.png".to_string(),
                    resource_type: "Texture2D".to_string(),
                    uid: Some("uid://dw612tw7iymyb".to_string()),
                },
            ),
            (
                "1_1jh5j".to_string(),
                godot_parser::ExternalResourceNode {
                    idx: 1,
                    id: "1_1jh5j".to_string(),
                    path: "res://scripts/background.gd".to_string(),
                    resource_type: "Script".to_string(),
                    uid: None,
                },
            ),
        ]),
        sub_resources: HashMap::from([(
            "Gradient_80myt".to_string(),
            godot_parser::SubResourceNode {
                idx: 0,
                id: "Gradient_80myt".to_string(),
                resource_type: "Gradient".to_string(),
                properties: HashMap::from([
                    (
                        "colors".to_string(),
                        godot_parser::OrderedProperty::new(
                            "PackedColorArray(0.98, 0.98, 0.98, 1, 0.81, 0.81, 0.81, 1)"
                                .to_string(),
                            0,
                        ),
                    ),
                    (
                        "offsets".to_string(),
                        godot_parser::OrderedProperty::new(
                            "PackedFloat32Array(0.0788732, 1)".to_string(),
                            1,
                        ),
                    ),
                ]),
            },
        )]),
        connections: HashMap::from([(
            "my_signal-node1-node2-my_method--".to_string(),
            GodotConnection {
                signal: "my_signal".to_string(),
                from_node_id: "node1".to_string(),
                to_node_id: "node2".to_string(),
                method: "my_method".to_string(),
                flags: None,
                unbinds: None,
                binds: None,
            },
        )]),
        requires_resave: false,
    };

    // write to automerge doc

    let mut doc = Automerge::new();
    let mut tx = doc.transaction();
    example_scene.reconcile(&mut tx, "example.tscn".to_string());
    tx.commit();

    // rehydrate from automerge doc

    let rehydrated_scene = GodotScene::hydrate(&mut doc, "example.tscn").unwrap();

    assert_eq!(example_scene, rehydrated_scene)
}


#[test]
fn test_node_paths() {
    let source = r#"[gd_scene load_steps=6 format=3 uid="uid://jnrusvm3gric"]
[node name="Root" type="Node2D"]
metadata/patchwork_id = "1122ae43c1054005997967892c521ea9"

[node name="Child" type="Node2D" parent="."]
metadata/patchwork_id = "1122ae43c1054005997967892c521ea2"

[node name="FindInFiles" parent="Child" node_paths=PackedStringArray("main_view", "code_edit") type="Node2D"]
metadata/patchwork_id = "1122ae43c1054005997967892c521ea3"
"#;

    let scene = godot_parser::parse_scene(&source.to_string()).unwrap();

    assert_eq!(scene.nodes.len(), 3);
    assert_eq!(scene.nodes["1122ae43c1054005997967892c521ea9"].node_paths, None);
    assert_eq!(scene.nodes["1122ae43c1054005997967892c521ea2"].node_paths, None);
    assert_eq!(scene.nodes["1122ae43c1054005997967892c521ea3"].node_paths, Some("PackedStringArray(\"main_view\", \"code_edit\")".to_string()));

}

#[test]
fn test_stringnames() {
    let source = r#"[gd_resource type="AudioBusLayout" format=3 uid="uid://c7thbop54thnf"]

[resource]
bus/1/name = &"Music"
bus/1/solo = false
bus/1/mute = false
bus/1/bypass_fx = false
bus/1/volume_db = 0.0
bus/1/send = &"Master"
bus/2/name = &"SFX"
bus/2/solo = false
bus/2/mute = false
bus/2/bypass_fx = false
bus/2/volume_db = 0.0
bus/2/send = &"Master"
prop:with_colon/2/name = &"Music"

"#;

    let resource = godot_parser::parse_scene(&source.to_string()).unwrap();

	assert_eq!(resource.main_resource.is_some(), true);
	let main_resource = resource.main_resource.as_ref().unwrap();
	let prop = main_resource.properties["bus/1/name"].clone();
	let ref_prop = godot_parser::OrderedProperty::new("&\"Music\"".to_string(), 0);
	assert_eq!(prop, ref_prop);

	let prop_with_colon = main_resource.properties["prop:with_colon/2/name"].clone();
	let ref_prop_with_colon = godot_parser::OrderedProperty::new("&\"Music\"".to_string(), 12);
	assert_eq!(prop_with_colon, ref_prop_with_colon);

	// reserialize and string compare
	let new_resource = resource.serialize();
	assert_eq!(new_resource, source);


}



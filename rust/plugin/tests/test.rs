use std::collections::HashMap;

use automerge::Automerge;
// Import the modules from the library
use patchwork_rust::godot_parser;
use patchwork_rust::godot_parser::GodotScene;
use patchwork_rust::godot_project;
use patchwork_rust::utils;

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
metadata/patchwork_id = "1122ae43c1054005997967892c521ea9"
script = ExtResource("1_1jh5j")
scroll_ignore_camera_zoom = true

[node name="ParallaxLayer" type="ParallaxLayer" parent="."]
metadata/patchwork_id = "ae876d398eb24a959b9ff1b00d983948"
motion_mirroring = Vector2(1600, 0)
motion_scale = Vector2(0, 0)
unique_name_in_owner = true

[node name="ColorRect" type="ColorRect" parent="ParallaxLayer"]
anchor_bottom = 1.0
anchor_right = 1.0
anchors_preset = 15
color = Color(0.980392, 0.980392, 0.980392, 1)
grow_horizontal = 2
grow_vertical = 2
metadata/patchwork_id = "5b9416e8d96042b6a509f7da3263f687"
offset_bottom = 98.0
offset_right = 5129.0
offset_top = -2352.0

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer"]
centered = false
metadata/patchwork_id = "50a6b8d7ce2c469098b3416372f9b1b8"
texture = SubResource("GradientTexture2D_ljotv")

[connection signal="body_entered" from="RigidBody2D" to="." method="_on_rigid_body_2d_body_entered"]
[connection signal="button_pressed" from="UI/Button" to="GameManager" method="_on_button_pressed" flags=3 unbinds=1 binds=["extra_param", 42, true]]
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

#[test]
fn test_resconcile_and_hydrate() {
    let source = get_test_scene_source();

    let example_scene = godot_parser::GodotScene {
        format: 3,
        load_steps: 0,
        uid: "uid://b8vp42c3k4q7v".to_string(),
        nodes: HashMap::from([
            (
                "node1".to_string(),
                godot_parser::GodotNode {
                    name: "Root".to_string(),
                    parent: None,
                    properties: HashMap::from([(
                        "metadata/patchwork_id".to_string(),
                        "node1".to_string(),
                    )]),
                    id: "node1".to_string(),
                    type_or_instance: godot_parser::TypeOrInstance::Type("Node2D".to_string()),
                    owner: None,
                    index: None,
                    groups: None,
                    child_node_ids: vec!["node2".to_string(), "node3".to_string()],
                },
            ),
            (
                "node2".to_string(),
                godot_parser::GodotNode {
                    name: "Sprite".to_string(),
                    parent: Some(".".to_string()),
                    properties: HashMap::from([
                        ("position".to_string(), "Vector2(100.0, 100.0)".to_string()),
                        ("metadata/patchwork_id".to_string(), "node2".to_string()),
                    ]),
                    id: "node2".to_string(),
                    type_or_instance: godot_parser::TypeOrInstance::Type("Sprite2D".to_string()),
                    owner: None,
                    index: None,
                    groups: None,
                    child_node_ids: vec![],
                },
            ),
            (
                "node3".to_string(),
                godot_parser::GodotNode {
                    name: "Label".to_string(),
                    parent: Some(".".to_string()),
                    properties: HashMap::from([
                        ("offset_right".to_string(), "40.0".to_string()),
                        ("offset_bottom".to_string(), "23.0".to_string()),
                        ("text".to_string(), "\"Hello World\"".to_string()),
                        ("metadata/patchwork_id".to_string(), "node3".to_string()),
                    ]),
                    id: "node3".to_string(),
                    type_or_instance: godot_parser::TypeOrInstance::Type("Label".to_string()),
                    owner: None,
                    index: None,
                    groups: None,
                    child_node_ids: vec![],
                },
            ),
        ]),
        root_node_id: "node1".to_string(),
        ext_resources: HashMap::from([
            (
                "1_0qn5k".to_string(),
                godot_parser::ExternalResourceNode {
                    id: "1_0qn5k".to_string(),
                    path: "res://assets/background-layer-1.png".to_string(),
                    resource_type: "Texture2D".to_string(),
                    uid: Some("uid://dw612tw7iymyb".to_string()),
                },
            ),
            (
                "1_1jh5j".to_string(),
                godot_parser::ExternalResourceNode {
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
                id: "Gradient_80myt".to_string(),
                resource_type: "Gradient".to_string(),
                properties: HashMap::from([
                    (
                        "colors".to_string(),
                        "PackedColorArray(0.98, 0.98, 0.98, 1, 0.81, 0.81, 0.81, 1)".to_string(),
                    ),
                    (
                        "offsets".to_string(),
                        "PackedFloat32Array(0.0788732, 1)".to_string(),
                    ),
                ]),
            },
        )]),
        connections: vec![],
    };

    // write to automerge doc

    let mut doc = Automerge::new();

    let mut tx = doc.transaction();

    example_scene.reconcile(&mut tx, "example.tscn".to_string());

    tx.commit();

    let rehydrated_scene = GodotScene::hydrate(&mut doc, "example.tscn").unwrap();

    let doc_json = serde_json::to_string_pretty(&automerge::AutoSerde::from(&doc)).unwrap();
    println!("Reconciled doc: {}", doc_json);

    // assert that rehydrated scene is deep equal to example scene
    assert_eq!(example_scene, rehydrated_scene);
}

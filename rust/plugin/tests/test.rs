use std::collections::HashMap;
use std::collections::HashSet;

use automerge::Automerge;
// Import the modules from the library
use patchwork_rust_core::godot_parser;
use patchwork_rust_core::godot_parser::GodotConnection;
use patchwork_rust_core::godot_parser::GodotScene;
use patchwork_rust_core::godot_project;
use patchwork_rust_core::utils;
use pretty_assertions::assert_eq;

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

[node name="NestedColorRect" type="ColorRect" parent="ParallaxLayer/ColorRect"]
anchor_bottom = 1.0
anchor_right = 1.0
anchors_preset = 15
color = Color(0.980392, 0.980392, 0.980392, 1)
grow_horizontal = 2
grow_vertical = 2
metadata/patchwork_id = "9a7c3e5b8f2d1a6c4b8e9d7f5a3c1e8b"
offset_bottom = 98.0
offset_right = 5129.0
offset_top = -2352.0

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer"]
centered = false
metadata/patchwork_id = "50a6b8d7ce2c469098b3416372f9b1b8"
texture = SubResource("GradientTexture2D_ljotv")

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

fn get_test_scene_source_with_duplicate_ids() -> String {
    return r#"[gd_scene load_steps=6 format=3 uid="uid://jnrusvm3gric"]
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
}

#[test]
fn test_parse_scene_with_duplicate_ids() {
    let source = get_test_scene_source_with_duplicate_ids();
    let scene = godot_parser::parse_scene(&source).unwrap();

    let root_node = scene.nodes.get(scene.root_node_id.as_str()).unwrap();

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
                    parent_id: Some("node1".to_string()),
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
                    parent_id: Some("node1".to_string()),
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
                        "PackedColorArray(0.98, 0.98, 0.98, 1, 0.81, 0.81, 0.81, 1)".to_string(),
                    ),
                    (
                        "offsets".to_string(),
                        "PackedFloat32Array(0.0788732, 1)".to_string(),
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
    };

    // write to automerge doc

    let mut doc = Automerge::new();

    let mut tx = doc.transaction();

    example_scene.reconcile(&mut tx, "example.tscn".to_string());

    tx.commit();

    let doc_clone = doc.clone();

    println!(
        "doc: {}",
        serde_json::to_string(&automerge::AutoSerde::from(&doc_clone)).unwrap()
    );

    let rehydrated_scene = GodotScene::hydrate(&mut doc, "example.tscn").unwrap();

    let doc_json = serde_json::to_string_pretty(&automerge::AutoSerde::from(&doc)).unwrap();
    println!("Reconciled doc: {}", doc_json);

    // assert that rehydrated scene is deep equal to example scene
    assert_eq!(example_scene, rehydrated_scene);
}

fn get_enemy_example_source() -> String {
    r#"[gd_scene load_steps=10 format=3 uid="uid://dk0xon0k7ga23"]

[ext_resource type="Texture2D" uid="uid://dwhi5vmkbnyk4" path="res://assets/enemy.png" id="1_57cf6"]
[ext_resource type="Script" path="res://components/enemy/enemy.gd" id="1_urf0o"]

[sub_resource type="AtlasTexture" id="AtlasTexture_5r7qd"]
atlas = ExtResource("1_57cf6")
region = Rect2(444, 0, 128, 128)

[sub_resource type="AtlasTexture" id="AtlasTexture_77x52"]
atlas = ExtResource("1_57cf6")
region = Rect2(0, 0, 128, 128)

[sub_resource type="AtlasTexture" id="AtlasTexture_emkhg"]
atlas = ExtResource("1_57cf6")
region = Rect2(148, 0, 128, 128)

[sub_resource type="AtlasTexture" id="AtlasTexture_ltgan"]
atlas = ExtResource("1_57cf6")
region = Rect2(296, 0, 128, 128)

[sub_resource type="RectangleShape2D" id="RectangleShape2D_cepcq"]
size = Vector2(82, 83)

[sub_resource type="RectangleShape2D" id="RectangleShape2D_kii40"]
size = Vector2(48, 78)

[sub_resource type="SpriteFrames" id="SpriteFrames_2xpc5"]
animations = [{
"frames": [{
"duration": 1.0,
"texture": SubResource("AtlasTexture_5r7qd")
}],
"loop": true,
"name": &"hit",
"speed": 5.0
}, {
"frames": [{
"duration": 1.0,
"texture": SubResource("AtlasTexture_77x52")
}, {
"duration": 1.0,
"texture": SubResource("AtlasTexture_emkhg")
}, {
"duration": 1.0,
"texture": SubResource("AtlasTexture_ltgan")
}],
"loop": true,
"name": &"walk",
"speed": 5.0
}]

[node name="Enemy" type="CharacterBody2D"]
collision_layer = 8
collision_mask = 4
floor_constant_speed = true
floor_snap_length = 32.0
metadata/patchwork_id = "3d7a6aa25c174300a7fc3e1f1e39d48c"
script = ExtResource("1_urf0o")

[node name="AnimatedSprite2D" type="AnimatedSprite2D" parent="."]
animation = "walk"
autoplay = "walk"
metadata/patchwork_id = "7a34982b78524765a6b843de8013bdaf"
position = Vector2(0, -64)
sprite_frames = SubResource("SpriteFrames_2xpc5")
unique_name_in_owner = true

[node name="LeftRay" type="RayCast2D" parent="."]
collision_mask = 5
metadata/patchwork_id = "5cec132b9b6b4d2bb750bfee34edfc51"
position = Vector2(-37, -3)
unique_name_in_owner = true

[node name="RightRay" type="RayCast2D" parent="."]
collision_mask = 5
metadata/patchwork_id = "fc9caf8886074a8ba88c9022596a75a9"
position = Vector2(37, -3)
unique_name_in_owner = true

[node name="CollisionShape2D" type="CollisionShape2D" parent="."]
metadata/patchwork_id = "c6151e8cb15b4f51a9cb89c136bd1cbf"
position = Vector2(0, -39)
shape = SubResource("RectangleShape2D_kii40")

[node name="Hitbox" type="Area2D" parent="."]
metadata/patchwork_id = "ec9415cd26d64578885c632b602b92e3"
position = Vector2(0, -106)

[node name="CollisionShape2D" type="CollisionShape2D" parent="Hitbox"]
metadata/patchwork_id = "c5e62e9ac3ae48589eac1ffcde9694a3"
position = Vector2(0, 58.5)
shape = SubResource("RectangleShape2D_cepcq")

[connection signal="body_entered" from="Hitbox" to="." method="_on_hitbox_body_entered"]
"#
    .to_string()
}

#[ignore]
#[test]
fn end_to_end_enemy_example() {
    let source = get_enemy_example_source();

    // Parse the scene
    let scene = godot_parser::parse_scene(&source).unwrap();

    // Write to automerge doc
    let mut doc = Automerge::new();
    let mut tx = doc.transaction();
    scene.reconcile(&mut tx, "enemy.tscn".to_string());
    tx.commit();

    // rehydrate scene
    let rehydrated_scene = GodotScene::hydrate(&mut doc, "enemy.tscn").unwrap();

    // serialize back to source
    let reserialized = rehydrated_scene.serialize();

    // Verify that the serialized output matches the original input
    assert_eq!(
        source, reserialized,
        "Serialized output should match original input"
    );
}

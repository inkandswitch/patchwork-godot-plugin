mod doc_utils;
mod godot_parser;

fn main() {
    let source = r#"[gd_scene load_steps=6 format=3 uid="uid://jnrusvm3gric"]

[ext_resource type="Texture2D" uid="uid://dw612tw7iymyb" path="res://assets/background-layer-1.png" id="1_0qn5k"]
[ext_resource type="Script" path="res://scripts/background.gd" id="1_1jh5j"]
[ext_resource type="Texture2D" uid="uid://dne1wh5fsffy" path="res://assets/background-layer-2.png" id="2_mk66l"]

[sub_resource type="Gradient" id="Gradient_80myt"]
offsets = PackedFloat32Array(0.0788732, 1)
colors = PackedColorArray(0.98, 0.98, 0.98, 1, 0.81, 0.81, 0.81, 1)

[sub_resource type="GradientTexture2D" id="GradientTexture2D_ljotv"]
gradient = SubResource("Gradient_80myt")
width = 5115
height = 1080
fill_to = Vector2(0, 1)

[node name="Background" type="ParallaxBackground"]
follow_viewport_enabled = true
scroll_ignore_camera_zoom = true
script = ExtResource("1_1jh5j")

[node name="ParallaxLayer" type="ParallaxLayer" parent="."]
unique_name_in_owner = true
motion_scale = Vector2(0, 0)
motion_mirroring = Vector2(1600, 0)

[node name="ColorRect" type="ColorRect" parent="ParallaxLayer"]
anchors_preset = 15
anchor_right = 1.0
anchor_bottom = 1.0
offset_top = -2352.0
offset_right = 5129.0
offset_bottom = 98.0
grow_horizontal = 2
grow_vertical = 2
color = Color(0.980392, 0.980392, 0.980392, 1)

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer"]
texture = SubResource("GradientTexture2D_ljotv")
centered = false

[node name="ParallaxLayer2" type="ParallaxLayer" parent="."]
unique_name_in_owner = true
motion_scale = Vector2(0.03, 0)
motion_mirroring = Vector2(1600, 0)

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer2"]
position = Vector2(0, -115)
scale = Vector2(2.5, 2.5)
texture = ExtResource("1_0qn5k")
centered = false

[node name="Sprite2D2" type="Sprite2D" parent="ParallaxLayer2"]
position = Vector2(1600, -115)
scale = Vector2(2.5, 2.5)
texture = ExtResource("1_0qn5k")
centered = false

[node name="ParallaxLayer3" type="ParallaxLayer" parent="."]
unique_name_in_owner = true
motion_scale = Vector2(0.1, 0)
motion_mirroring = Vector2(1600, 0)

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer3"]
position = Vector2(0, -115)
scale = Vector2(2.5, 2.5)
texture = ExtResource("2_mk66l")
centered = false

[node name="Sprite2D2" type="Sprite2D" parent="ParallaxLayer3"]
position = Vector2(1600, -115)
scale = Vector2(2.5, 2.5)
texture = ExtResource("2_mk66l")
centered = false
"#;

    let scene = godot_parser::parse_scene(&source.to_string()).unwrap();

    // println!("{:#?}", scene);

    let reserialized: String = scene.serialize();
    println!("{}", reserialized);

    println!("check: {}", source == reserialized);
}

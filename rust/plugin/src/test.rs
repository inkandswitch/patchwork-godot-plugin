mod doc_utils;
mod godot_parser;

fn main() {
    let source = r#"[gd_scene load_steps=6 format=3 uid="uid://jnrusvm3gric"]

[ext_resource type="Texture2D" uid="uid://dw612tw7iymyb" path="res://assets/background-layer-1.png" id="1_0qn5k"]
[ext_resource type="Script" path="res://scripts/background.gd" id="1_1jh5j"]
[ext_resource type="Texture2D" uid="uid://dne1wh5fsffy" path="res://assets/background-layer-2.png" id="2_mk66l"]

[sub_resource type="Gradient" id="Gradient_80myt"]
colors=PackedColorArray(0.98, 0.98, 0.98, 1, 0.81, 0.81, 0.81, 1)
offsets=PackedFloat32Array(0.0788732, 1)

[sub_resource type="GradientTexture2D" id="GradientTexture2D_ljotv"]
fill_to=Vector2(0, 1)
gradient=SubResource("Gradient_80myt")
height=1080
width=5115

[node name="Background" type="ParallaxBackground"]
follow_viewport_enabled=true
metadata/patchwork_id="1122ae43c1054005997967892c521ea9"
script=ExtResource("1_1jh5j")
scroll_ignore_camera_zoom=true

[node name="ParallaxLayer" type="ParallaxLayer" parent="."]
metadata/patchwork_id="ae876d398eb24a959b9ff1b00d983948"
motion_mirroring=Vector2(1600, 0)
motion_scale=Vector2(0, 0)
unique_name_in_owner=true

[node name="ColorRect" type="ColorRect" parent="ParallaxLayer"]
anchor_bottom=1.0
anchor_right=1.0
anchors_preset=15
color=Color(0.980392, 0.980392, 0.980392, 1)
grow_horizontal=2
grow_vertical=2
metadata/patchwork_id="5b9416e8d96042b6a509f7da3263f687"
offset_bottom=98.0
offset_right=5129.0
offset_top=-2352.0

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer"]
centered=false
metadata/patchwork_id="50a6b8d7ce2c469098b3416372f9b1b8"
texture=SubResource("GradientTexture2D_ljotv")

[node name="ParallaxLayer2" type="ParallaxLayer" parent="."]
metadata/patchwork_id="68e34737ea7945149341e96b6f3172de"
motion_mirroring=Vector2(1600, 0)
motion_scale=Vector2(0.03, 0)
unique_name_in_owner=true

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer2"]
centered=false
metadata/patchwork_id="9efb44401db342208b03bc4c40339637"
position=Vector2(0, -115)
scale=Vector2(2.5, 2.5)
texture=ExtResource("1_0qn5k")

[node name="Sprite2D2" type="Sprite2D" parent="ParallaxLayer2"]
centered=false
metadata/patchwork_id="0c3d57c30d5d47079aad9a8c248666c1"
position=Vector2(1600, -115)
scale=Vector2(2.5, 2.5)
texture=ExtResource("1_0qn5k")

[node name="ParallaxLayer3" type="ParallaxLayer" parent="."]
metadata/patchwork_id="3a85ca7948fe49b4b3c8a719093b61dc"
motion_mirroring=Vector2(1600, 0)
motion_scale=Vector2(0.1, 0)
unique_name_in_owner=true

[node name="Sprite2D" type="Sprite2D" parent="ParallaxLayer3"]
centered=false
metadata/patchwork_id="9384679045484b848d852999d3e3b4a3"
position=Vector2(0, -115)
scale=Vector2(2.5, 2.5)
texture=ExtResource("2_mk66l")

[node name="Sprite2D2" type="Sprite2D" parent="ParallaxLayer3"]
centered=false
metadata/patchwork_id="49876c7811a94d099d6afdb50024da0e"
position=Vector2(1600, -115)
scale=Vector2(2.5, 2.5)
texture=ExtResource("2_mk66l")

[connection signal="body_entered" from="RigidBody2D" to="." method="_on_rigid_body_2d_body_entered"]
[connection signal="button_pressed" from="UI/Button" to="GameManager" method="_on_button_pressed" flags=3 unbinds=1 binds=["extra_param", 42, true]]
"#;

    let scene = godot_parser::parse_scene(&source.to_string()).unwrap();

    // println!("{:#?}", scene);

    let reserialized: String = scene.serialize();
    println!("'{}'", reserialized);
    println!("check: {}", source == reserialized);
}

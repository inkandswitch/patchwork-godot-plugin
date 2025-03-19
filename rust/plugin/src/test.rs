mod doc_utils;
mod godot_parser;

fn main() {
    let example = r#"[gd_scene load_steps=9 format=3 uid="uid://bel3lwsvc2uc3"]

[ext_resource type="PackedScene" uid="uid://s7enbp56f256" path="res://components/paddles/basic_paddle.tscn" id="1_xxpg5"]
[ext_resource type="PackedScene" uid="uid://cg1m4evi8c35o" path="res://components/other/spawner.tscn" id="3_spct4"]
[ext_resource type="PackedScene" uid="uid://bn368pidqsqgs" path="res://rules_goals/game_logic.tscn" id="3_umelw"]
[ext_resource type="PackedScene" uid="uid://bis7afjjuwypq" path="res://hud.tscn" id="4_4868o"]
[ext_resource type="PackedScene" uid="uid://cg8ibi18um3vg" path="res://spaces/basic_space.tscn" id="4_nvjpm"]
[ext_resource type="PackedScene" uid="uid://c7l70grmkauij" path="res://components/balls/basic_ball.tscn" id="4_rffv2"]

[sub_resource type="RectangleShape2D" id="RectangleShape2D_guyqd"]
size = Vector2(10, 1080)

[sub_resource type="SystemFont" id="SystemFont_24cl0"]

[node name="Main" type="Node2D"]
metadata/_edit_vertical_guides_ = [-933.0]

[node name="BasicSpace" parent="." instance=ExtResource("4_nvjpm")]

[node name="BasicPaddleLeft" parent="." instance=ExtResource("1_xxpg5")]
position = Vector2(64, 512)
tint = Color(0.511023, 0.361749, 0.971889, 1)

[node name="BasicPaddleRight" parent="." instance=ExtResource("1_xxpg5")]
position = Vector2(1856, 512)
player = 1
tint = Color(0.511023, 0.361749, 0.971889, 1)

[node name="BallSpawner" parent="." groups=["ball spawners"] instance=ExtResource("3_spct4")]
position = Vector2(967, 531)
spawn_area = SubResource("RectangleShape2D_guyqd")
life_time = 0.0

[node name="BasicBall" parent="BallSpawner" instance=ExtResource("4_rffv2")]
linear_velocity = Vector2(353.553, 353.553)
tint = Color(0.509804, 0.360784, 0.972549, 1)

[node name="GameLogic" parent="." instance=ExtResource("3_umelw")]

[node name="HUD" parent="." instance=ExtResource("4_4868o")]
font = SubResource("SystemFont_24cl0")"#;

    let scene = godot_parser::parse(&example.to_string());
    println!("{:?}", scene);
}

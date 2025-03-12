mod doc_utils;
mod godot_parser;

fn main() {
    let example = r#"[gd_scene]

[ext_resource uid="uid://cg8ibi18um3vg" path="res://spaces/basic_space.tscn" type="PackedScene" id="4_nvjpm"]
[ext_resource type="PackedScene" path="res://components/other/spawner.tscn" uid="uid://cg1m4evi8c35o" id="3_spct4"]
[ext_resource uid="uid://c7l70grmkauij" path="res://components/balls/basic_ball.tscn" type="PackedScene" id="4_rffv2"]
[ext_resource path="res://components/paddles/basic_paddle.tscn" type="PackedScene" uid="uid://s7enbp56f256" id="1_xxpg5"]
[ext_resource type="PackedScene" path="res://hud.tscn" uid="uid://bis7afjjuwypq" id="4_4868o"]
[ext_resource type="PackedScene" path="res://rules_goals/game_logic.tscn" uid="uid://bn368pidqsqgs" id="3_umelw"]

[sub_resource type="SystemFont" id="SystemFont_24cl0"]

[sub_resource type="RectangleShape2D" id="RectangleShape2D_guyqd"]
size=Vector2(10, 1080)

[node name="Main" type="Node2D"]
metadata/patchwork_id=07261d3c183548b2bb506e626fd76215
metadata/_edit_vertical_guides_=[-933.0]

[node name="BasicSpace" parent="Main" instance=ExtResource("4_nvjpm")]
metadata/patchwork_id=040d674c60864e73b75ec0d4a7cb5fa6

[node name="BasicPaddleLeft" parent="Main" instance=ExtResource("1_xxpg5")]
metadata/patchwork_id=6e22080ab06c43dd8ee2697701a394d6
position=Vector2(64, 512)
tint=Color(0.511023, 0.361749, 0.971889, 1)

[node name="BasicPaddleRight" parent="Main" instance=ExtResource("1_xxpg5")]
position=Vector2(1856, 512)
player=1
tint=Color(0.511023, 0.361749, 0.971889, 1)
metadata/patchwork_id=cbea05a277824aab8a4ac9c1ca465624

[node name="BallSpawner" parent="Main" groups=["ball spawners"] instance=ExtResource("3_spct4")]
life_time=0.0
spawn_area=SubResource("RectangleShape2D_guyqd")
position=Vector2(967, 531)
metadata/patchwork_id=64ff6361f11c4928af8e5ffe1b57b5f5

[node name="BasicBall" parent="Main/BallSpawner" instance=ExtResource("4_rffv2")]
tint=Color(0.509804, 0.360784, 0.972549, 1)
linear_velocity=Vector2(353.553, 353.553)
metadata/patchwork_id=698e6e9587734cf08c0380c2d309f0a9

[node name="GameLogic" parent="Main" instance=ExtResource("3_umelw")]
metadata/patchwork_id=9a5a1feba60e4958935a717d2e60562f

[node name="HUD" parent="Main" instance=ExtResource("4_4868o")]
metadata/patchwork_id=1a9bd9ba6675481ca263663ee7ef7049
font=SubResource("SystemFont_24cl0")"#;

    let scene = godot_parser::parse_scene(&example.to_string()).unwrap();

    println!("{:#?}", scene);

    let source = scene.serialize();
    println!("{}", source);

    println!("check: {}", source == example);
}

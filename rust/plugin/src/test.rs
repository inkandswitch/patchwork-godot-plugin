mod doc_utils;
mod godot_parser;

fn main() {
    let example = r#"[gd_scene load_steps=14 format=4 uid="uid://dhcpt1kt8cs0g"]

[ext_resource type="PackedScene" uid="uid://8st4scqt06l8" path="res://components/player/player.tscn" id="2_7yl00"]
[ext_resource type="PackedScene" uid="uid://jnrusvm3gric" path="res://spaces/background.tscn" id="2_tb5a2"]
[ext_resource type="PackedScene" uid="uid://cswheshouik14" path="res://rules_goals/game_logic.tscn" id="3_xbkvd"]
[ext_resource type="PackedScene" uid="uid://danlrmsfmsros" path="res://spaces/tilemap.tscn" id="3_yfnmv"]
[ext_resource type="PackedScene" uid="uid://dthyncc3spfks" path="res://spaces/dangerzone.tscn" id="4_2mg6t"]
[ext_resource type="PackedScene" uid="uid://coq6d3u6wnvs2" path="res://components/platform/platform.tscn" id="4_gd51l"]
[ext_resource type="PackedScene" uid="uid://daf24t18h3n5e" path="res://components/coin/coin.tscn" id="5_u7hr5"]
[ext_resource type="Texture2D" uid="uid://bjqboxowe07lw" path="res://assets/items/crystal.png" id="6_kpi5m"]
[ext_resource type="PackedScene" uid="uid://jt80gv02u4f2" path="res://hud.tscn" id="6_mp7wy"]
[ext_resource type="PackedScene" uid="uid://dk0xon0k7ga23" path="res://components/enemy/enemy.tscn" id="9_l6smt"]
[ext_resource type="SpriteFrames" uid="uid://bo581k1esb50n" path="res://components/player/spriteframes-red.tres" id="9_qmofe"]
[ext_resource type="PackedScene" uid="uid://beuisy5yrw0bq" path="res://components/flag/flag.tscn" id="12_dkbog"]
[ext_resource type="Script" path="res://scripts/multiplayer_camera.gd" id="13_0d2mj"]

[node name="Main" type="Node2D"]

[node name="GameLogic" parent="." instance=ExtResource("3_xbkvd")]
win_by_collecting_coins = true
win_by_reaching_flag = true

[node name="Background" parent="." instance=ExtResource("2_tb5a2")]
tint = Color(0.569993, 0.558956, 0.878091, 1)

[node name="Dangerzone" parent="." instance=ExtResource("4_2mg6t")]
position = Vector2(3072, 1216)

[node name="Coins" type="Node2D" parent="."]

[node name="Coin" parent="Coins" instance=ExtResource("5_u7hr5")]
modulate = Color(1, 1, 0, 1)
position = Vector2(1472, 320)
texture = ExtResource("6_kpi5m")
tint = Color(1, 1, 0, 1)

[node name="Coin3" parent="Coins" instance=ExtResource("5_u7hr5")]
modulate = Color(1, 1, 0, 1)
position = Vector2(1600, 256)
texture = ExtResource("6_kpi5m")
tint = Color(1, 1, 0, 1)

[node name="Coin4" parent="Coins" instance=ExtResource("5_u7hr5")]
modulate = Color(1, 1, 0, 1)
position = Vector2(1728, 320)
texture = ExtResource("6_kpi5m")
tint = Color(1, 1, 0, 1)

[node name="Platforms" type="Node2D" parent="."]
position = Vector2(1920, -64)

[node name="Platform" parent="Platforms" instance=ExtResource("4_gd51l")]
position = Vector2(960, 320)
width = 2

[node name="Platform4" parent="Platforms" instance=ExtResource("4_gd51l")]
position = Vector2(-1472, 0)
width = 2
one_way = true
fall_time = 2.0

[node name="Player" parent="." instance=ExtResource("2_7yl00")]
position = Vector2(512, 576)
collision_layer = 1
collision_mask = 7
sprite_frames = ExtResource("9_qmofe")

[node name="Camera2D" type="Camera2D" parent="Player"]
position = Vector2(0, 15)
limit_left = 0
limit_bottom = 1080
position_smoothing_enabled = true

[node name="HUD" parent="." instance=ExtResource("6_mp7wy")]

[node name="Enemy" parent="." instance=ExtResource("9_l6smt")]
position = Vector2(1600, 704)

[node name="Enemy2" parent="." instance=ExtResource("9_l6smt")]
position = Vector2(1920, 576)
"#;

    let reserialized = r#"[gd_scene load_steps=14 format=4 uid="uid://dhcpt1kt8cs0g"]

[ext_resource uid="uid://jnrusvm3gric" type="PackedScene" path="res://spaces/background.tscn" id="2_tb5a2"]
[ext_resource uid="uid://dthyncc3spfks" type="PackedScene" path="res://spaces/dangerzone.tscn" id="4_2mg6t"]
[ext_resource path="res://components/player/spriteframes-red.tres" uid="uid://bo581k1esb50n" type="SpriteFrames" id="9_qmofe"]
[ext_resource type="PackedScene" uid="uid://8st4scqt06l8" path="res://components/player/player.tscn" id="2_7yl00"]
[ext_resource uid="uid://jt80gv02u4f2" type="PackedScene" path="res://hud.tscn" id="6_mp7wy"]
[ext_resource type="Script" path="res://scripts/multiplayer_camera.gd" id="13_0d2mj"]
[ext_resource type="PackedScene" uid="uid://cswheshouik14" path="res://rules_goals/game_logic.tscn" id="3_xbkvd"]
[ext_resource type="Texture2D" path="res://assets/items/crystal.png" uid="uid://bjqboxowe07lw" id="6_kpi5m"]
[ext_resource type="PackedScene" uid="uid://beuisy5yrw0bq" path="res://components/flag/flag.tscn" id="12_dkbog"]
[ext_resource path="res://spaces/tilemap.tscn" type="PackedScene" uid="uid://danlrmsfmsros" id="3_yfnmv"]
[ext_resource type="PackedScene" uid="uid://daf24t18h3n5e" path="res://components/coin/coin.tscn" id="5_u7hr5"]
[ext_resource uid="uid://coq6d3u6wnvs2" type="PackedScene" path="res://components/platform/platform.tscn" id="4_gd51l"]
[ext_resource type="PackedScene" uid="uid://dk0xon0k7ga23" path="res://components/enemy/enemy.tscn" id="9_l6smt"]

[node name="Main" type="Node2D"]
metadata/patchwork_id="cb69ec7e34864e5ca2bd4d0ca7105834"

[node name="GameLogic" parent="." instance=ExtResource("3_xbkvd")]
win_by_reaching_flag=true
metadata/patchwork_id="d371a8c3887747538234d2300e26fc1f"
win_by_collecting_coins=true

[node name="Background" parent="." instance=ExtResource("2_tb5a2")]
metadata/patchwork_id="14d0420c1c0844b582d2e6fac7569249"
tint=Color(0.569993, 0.558956, 0.878091, 1)

[node name="Dangerzone" parent="." instance=ExtResource("4_2mg6t")]
metadata/patchwork_id="4f8f5c8d862e4793b3e1c10bcee8db75"
position=Vector2(3072, 1216)

[node name="Coins" type="Node2D" parent="."]
metadata/patchwork_id="6b9b86bf04b64b958a043bc3444f2d88"

[node name="Coin" parent="Coins" instance=ExtResource("5_u7hr5")]
texture=ExtResource("6_kpi5m")
metadata/patchwork_id="486819e024244c9880bee32c0a846a08"
position=Vector2(1472, 320)
tint=Color(1, 1, 0, 1)
modulate=Color(1, 1, 0, 1)

[node name="Coin3" parent="Coins" instance=ExtResource("5_u7hr5")]
modulate=Color(1, 1, 0, 1)
tint=Color(1, 1, 0, 1)
position=Vector2(1600, 256)
metadata/patchwork_id="ff095b825e7642739a37e2329f1e30fb"
texture=ExtResource("6_kpi5m")

[node name="Coin4" parent="Coins" instance=ExtResource("5_u7hr5")]
metadata/patchwork_id="48e4d518129f4cdaad74075a7ae458f1"
texture=ExtResource("6_kpi5m")
position=Vector2(1728, 320)
modulate=Color(1, 1, 0, 1)
tint=Color(1, 1, 0, 1)

[node name="Platforms" type="Node2D" parent="."]
metadata/patchwork_id="a780e1716d0641e5a6577a23f3d75932"
position=Vector2(1920, -64)

[node name="Platform" parent="Platforms" instance=ExtResource("4_gd51l")]
width=2
metadata/patchwork_id="a9c3a5a969054291a512136c4baee35a"
position=Vector2(960, 320)

[node name="Platform4" parent="Platforms" instance=ExtResource("4_gd51l")]
width=2
fall_time=2.0
one_way=true
metadata/patchwork_id="ee10dc07f8fa42b29aa5b3249b6758d8"
position=Vector2(-1472, 0)

[node name="Player" parent="." instance=ExtResource("2_7yl00")]
sprite_frames=ExtResource("9_qmofe")
position=Vector2(512, 576)
collision_layer=1
metadata/patchwork_id="6c9f6bc542514ca18fc1a24c0a734a19"
collision_mask=7

[node name="Camera2D" type="Camera2D" parent="Player"]
metadata/patchwork_id="31ae337cfea242bfada4d2a1786b32f1"
position_smoothing_enabled=true
limit_bottom=1080
limit_left=0
position=Vector2(0, 15)

[node name="HUD" parent="." instance=ExtResource("6_mp7wy")]
metadata/patchwork_id="a7f5c2a130bf4602b589000a672769c7"

[node name="Enemy" parent="." instance=ExtResource("9_l6smt")]
metadata/patchwork_id="b0927783ef0a4241b1ffa30f86bf63fa"
position=Vector2(1600, 704)

[node name="Enemy2" parent="." instance=ExtResource("9_l6smt")]
position=Vector2(1920, 576)
metadata/patchwork_id="431d07788c4242d6a54b7bea25dd2357"
"#;

    let scene = godot_parser::parse_scene(&reserialized.to_string()).unwrap();

    // println!("{:#?}", scene);

    let source = scene.serialize();
    println!("{}", source);

    println!("check: {}", source == reserialized);
}

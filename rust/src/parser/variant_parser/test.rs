use crate::parser::parser_defs::{ElemType, RealT, VariantVal};
use indexmap::IndexMap;

fn test_cases() -> Vec<(&'static str, VariantVal, bool)> {
    let mut map_dict = IndexMap::new();
    map_dict.insert(
        Box::new(VariantVal::String("foo".into())),
        Box::new(VariantVal::String("bar".into())),
    );
    map_dict.insert(
        Box::new(VariantVal::String("baz".into())),
        Box::new(VariantVal::Int(123)),
    );

    let mut map_typed_dict = IndexMap::new();
    map_typed_dict.insert(
        Box::new(VariantVal::String("foo".into())),
        Box::new(VariantVal::Int(123)),
    );
    map_typed_dict.insert(
        Box::new(VariantVal::String("baz".into())),
        Box::new(VariantVal::Int(456)),
    );

    let mut object_props = IndexMap::new();
    object_props.insert("bar".into(), Box::new(VariantVal::Int(123)));

    vec![
        (
            "Vector2(-1, -2)",
            VariantVal::Vector2(RealT::F32(-1.0), RealT::F32(-2.0)),
            true,
        ),
        ("null", VariantVal::Nil, true),
        ("nil", VariantVal::Nil, false),
        ("true", VariantVal::Bool(true), true),
        ("false", VariantVal::Bool(false), true),
        ("123", VariantVal::Int(123), true),
        ("123.0", VariantVal::Float(123.0), true),
        ("123.456", VariantVal::Float(123.456), true),
        ("1.5707964", VariantVal::Float(1.5707964), true),
        // scientific notation
        ("1.23456e+10", VariantVal::Float(1.23456e+10), true),
        ("1.23456e-10", VariantVal::Float(1.23456e-10), true),
        ("inf", VariantVal::Float(f64::INFINITY), true),
        ("-inf", VariantVal::Float(f64::NEG_INFINITY), true),
        ("nan", VariantVal::Float(f64::NAN), true),
        ("\"foo\"", VariantVal::String("foo".into()), true),
        ("&\"foo\"", VariantVal::StringName("foo".into()), true),
        ("#ff0000", VariantVal::Color(1.0, 0.0, 0.0, 1.0), false),
        (
            "#ff000080",
            VariantVal::Color(1.0, 0.0, 0.0, 128.0 / 255.0),
            false,
        ),
        ("#f00", VariantVal::Color(1.0, 0.0, 0.0, 1.0), false), // 3-digit (Godot Color::html)
        ("#f008", VariantVal::Color(1.0, 0.0, 0.0, 8.0 / 15.0), false), // 4-digit
        (
            "Vector2(1, 2)",
            VariantVal::Vector2(RealT::F32(1.0), RealT::F32(2.0)),
            true,
        ),
        ("Vector2i(1, 2)", VariantVal::Vector2i(1, 2), true),
        (
            "Rect2(0, 0, 10, 10)",
            VariantVal::Rect2(
                (RealT::F64(0.0), RealT::F64(0.0)),
                (RealT::F64(10.0), RealT::F64(10.0)),
            ),
            true,
        ),
        (
            "Rect2i(0, 0, 10, 10)",
            VariantVal::Rect2i((0, 0), (10, 10)),
            true,
        ),
        (
            "Vector3(1, 2, 3)",
            VariantVal::Vector3(RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0)),
            true,
        ),
        ("Vector3i(1, 2, 3)", VariantVal::Vector3i(1, 2, 3), true),
        (
            "Vector4(1, 2, 3, 4)",
            VariantVal::Vector4(
                RealT::F64(1.0),
                RealT::F64(2.0),
                RealT::F64(3.0),
                RealT::F64(4.0),
            ),
            true,
        ),
        (
            "Vector4i(1, 2, 3, 4)",
            VariantVal::Vector4i(1, 2, 3, 4),
            true,
        ),
        (
            "Transform2D(1, 0, 0, 1, 0, 0)",
            VariantVal::Transform2d(
                (RealT::F64(1.0), RealT::F64(0.0)),
                (RealT::F64(0.0), RealT::F64(1.0)),
                (RealT::F64(0.0), RealT::F64(0.0)),
            ),
            true,
        ),
        (
            "Plane(1, 0, 0, 0)",
            VariantVal::Plane(
                (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0)),
                RealT::F64(0.0),
            ),
            true,
        ),
        (
            "Quaternion(1, 0, 0, 0)",
            VariantVal::Quaternion(
                RealT::F64(1.0),
                RealT::F64(0.0),
                RealT::F64(0.0),
                RealT::F64(0.0),
            ),
            true,
        ),
        (
            "AABB(0, 0, 0, 1, 1, 1)",
            VariantVal::Aabb(
                (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(0.0)),
                (RealT::F64(1.0), RealT::F64(1.0), RealT::F64(1.0)),
            ),
            true,
        ),
        (
            "Basis(1, 0, 0, 0, 1, 0, 0, 0, 1)",
            VariantVal::Basis(
                (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0)),
                (RealT::F64(0.0), RealT::F64(1.0), RealT::F64(0.0)),
                (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(1.0)),
            ),
            true,
        ),
        (
            "Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0)",
            VariantVal::Transform3d(
                (
                    (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0)),
                    (RealT::F64(0.0), RealT::F64(1.0), RealT::F64(0.0)),
                    (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(1.0)),
                ),
                (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(0.0)),
            ),
            true,
        ),
        (
            "Color(1, 0, 0, 1)",
            VariantVal::Color(1.0, 0.0, 0.0, 1.0),
            true,
        ),
        (
            "NodePath(\"foo/bar/baz\")",
            VariantVal::NodePath("foo/bar/baz".into()),
            true,
        ),
        ("RID()", VariantVal::Rid("".into()), true),
        ("RID(42)", VariantVal::Rid("42".into()), true),
        ("Callable()", VariantVal::Callable, true),
        ("Signal()", VariantVal::Signal, true),
        (
            "Object(Node, \"bar\": 123)",
            VariantVal::Object("Node".into(), object_props),
            true,
        ),
        (
            "{\n\"foo\": \"bar\",\n\"baz\": 123\n}",
            VariantVal::Dictionary(None, map_dict),
            true,
        ),
        (
            "[1, 2, 3]",
            VariantVal::Array(
                None,
                vec![
                    Box::new(VariantVal::Int(1)),
                    Box::new(VariantVal::Int(2)),
                    Box::new(VariantVal::Int(3)),
                ],
            ),
            true,
        ),
        (
            "Dictionary[String, int]({\n\"foo\": 123,\n\"baz\": 456\n})",
            VariantVal::Dictionary(
                Some((
                    Box::new(ElemType::Identifier("String".into())),
                    Box::new(ElemType::Identifier("int".into())),
                )),
                map_typed_dict,
            ),
            true,
        ),
        (
            "Array[int]([1, 2, 3])",
            VariantVal::Array(
                Some(Box::new(ElemType::Identifier("int".into()))),
                vec![
                    Box::new(VariantVal::Int(1)),
                    Box::new(VariantVal::Int(2)),
                    Box::new(VariantVal::Int(3)),
                ],
            ),
            true,
        ),
        (
            "PackedByteArray(0, 0, 0, 0, 0)",
            VariantVal::PackedByteArray(vec![0, 0, 0, 0, 0]),
            false,
        ),
        (
            "PackedByteArray(\"AAAAAAA=\")",
            VariantVal::PackedByteArray(vec![0, 0, 0, 0, 0]),
            true,
        ),
        (
            "PackedInt32Array(1, 2, 3)",
            VariantVal::PackedInt32Array(vec![1, 2, 3]),
            true,
        ),
        (
            "PackedInt64Array(1, 2, 3)",
            VariantVal::PackedInt64Array(vec![1, 2, 3]),
            true,
        ),
        (
            "PackedFloat32Array(1, 2, 3)",
            VariantVal::PackedFloat32Array(vec![1.0, 2.0, 3.0]),
            true,
        ),
        (
            "PackedFloat64Array(1, 2, 3)",
            VariantVal::PackedFloat64Array(vec![1.0, 2.0, 3.0]),
            true,
        ),
        (
            "PackedStringArray(\"a\", \"b\", \"c\")",
            VariantVal::PackedStringArray(vec!["a".into(), "b".into(), "c".into()]),
            true,
        ),
        (
            "PackedVector2Array(1, 2, 3, 4)",
            VariantVal::PackedVector2Array(vec![
                (RealT::F64(1.0), RealT::F64(2.0)),
                (RealT::F64(3.0), RealT::F64(4.0)),
            ]),
            true,
        ),
        (
            "PackedVector3Array(1, 2, 3, 4, 5, 6)",
            VariantVal::PackedVector3Array(vec![
                (RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0)),
                (RealT::F64(4.0), RealT::F64(5.0), RealT::F64(6.0)),
            ]),
            true,
        ),
        (
            "PackedVector4Array(1, 2, 3, 4, 5, 6, 7, 8)",
            VariantVal::PackedVector4Array(vec![
                (
                    RealT::F64(1.0),
                    RealT::F64(2.0),
                    RealT::F64(3.0),
                    RealT::F64(4.0),
                ),
                (
                    RealT::F64(5.0),
                    RealT::F64(6.0),
                    RealT::F64(7.0),
                    RealT::F64(8.0),
                ),
            ]),
            true,
        ),
        (
            "PackedColorArray(1, 0, 0, 1, 0, 1, 0, 1)",
            VariantVal::PackedColorArray(vec![
                (
                    RealT::F64(1.0),
                    RealT::F64(0.0),
                    RealT::F64(0.0),
                    RealT::F64(1.0),
                ),
                (
                    RealT::F64(0.0),
                    RealT::F64(1.0),
                    RealT::F64(0.0),
                    RealT::F64(1.0),
                ),
            ]),
            true,
        ),
        (
            "Resource(\"res://bar.tres\")",
            VariantVal::Resource(None, "res://bar.tres".into()),
            true,
        ),
        (
            "Resource(\"uid://5252525252\", \"res://bar.tres\")",
            VariantVal::Resource(Some("uid://5252525252".into()), "res://bar.tres".into()),
            true,
        ),
        (
            "SubResource(\"foo\")",
            VariantVal::SubResource("foo".into()),
            true,
        ),
        (
            "ExtResource(\"1_ffe31\")",
            VariantVal::ExtResource("1_ffe31".into()),
            true,
        ),
    ]
}

#[test]
fn test_every_variant_type() {
    for (input, expected, compare_string) in test_cases() {
        let parsed = input.parse::<VariantVal>().unwrap_or_else(|e| {
            panic!("Failed to parse {:?}: {}", input, e);
        });
        assert_eq!(parsed, expected, "input: {:?}", input);
        if compare_string {
            assert_eq!(
                expected.to_string_compat(false).unwrap(),
                input,
                "input: {:?}",
                input
            );
        }
    }
}

/// Writer and parser Variant::FLOAT (mirrors Godot test_variant.h).
/// Variant::FLOAT is always 64-bit (f64). Tests max finite double write/parse round-trip.
#[test]
fn test_writer_and_parser_float() {
    // Maximum non-infinity double-precision float (same as C++ test).
    let a64: f64 = f64::MAX;
    let a64_str = VariantVal::Float(a64).to_string_compat(true).unwrap();

    assert_eq!(
        a64_str, "1.7976931348623157e+308",
        "Writes in scientific notation."
    );
    assert_ne!(a64_str, "inf", "Should not overflow.");
    assert_ne!(a64_str, "nan", "The result should be defined.");

    // Parse back; loses precision in string form but round-trip value is correct.
    let variant_parsed: VariantVal = a64_str.parse().expect("parse max float");
    let float_parsed = match &variant_parsed {
        VariantVal::Float(f) => *f,
        _ => panic!("expected Float, got {:?}", variant_parsed),
    };
    let expected: f64 = 1.797693134862315708145274237317e+308;
    assert_eq!(
        float_parsed.to_bits(),
        expected.to_bits(),
        "Should parse back."
    );

    // Approximation of Googol with double-precision float.
    let variant_parsed: VariantVal = "1.0e+100".parse().expect("parse 1.0e+100");
    let float_parsed = match &variant_parsed {
        VariantVal::Float(f) => *f,
        _ => panic!("expected Float, got {:?}", variant_parsed),
    };
    assert_eq!(float_parsed, 1.0e+100, "Should match the double literal.");
}

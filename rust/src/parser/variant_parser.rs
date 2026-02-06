use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use indexmap::IndexMap;

use base64::Engine;
use godot::{builtin::*, meta::ToGodot, prelude::GodotConvert};
use lexical::{NumberFormatBuilder, WriteFloatOptions};
use num_traits::{Zero, Float};


// -----------------------------------------------------------------------------
// Parse error
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VariantParseError(pub String);

impl Display for VariantParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VariantParseError {}

// -----------------------------------------------------------------------------
// RealT (f32/f64 for Godot real_t)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum RealT {
    F32(f32),
    F64(f64),
}

impl RealT {
    pub fn to_f32(&self) -> f32 {
        match self {
            RealT::F32(f) => *f,
            RealT::F64(f) => *f as f32,
        }
    }
    pub fn to_f64(&self) -> f64 {
        match self {
            RealT::F32(f) => *f as f64,
            RealT::F64(f) => *f,
        }
    }
}

impl From<f64> for RealT {
    fn from(f: f64) -> Self {
        RealT::F64(f)
    }
}

// -----------------------------------------------------------------------------
// Float formatting (Godot rtos_fix / num_scientific via lexical Grisu2)
// -----------------------------------------------------------------------------

const LEXICAL_FORMAT: u128 = NumberFormatBuilder::new()
    .required_exponent_sign(true)
    .build_strict();

/// Trait to call lexical's formatter with the correct type (f32 vs f64). Rust cannot
/// compare generic `T` to a type like `f32`, so we dispatch via this trait instead.
trait FormatFloatLexical {
    fn format_float_lexical(self, options: &WriteFloatOptions) -> String;
}
impl FormatFloatLexical for f32 {
    fn format_float_lexical(self, options: &WriteFloatOptions) -> String {
        lexical::to_string_with_options::<f32, LEXICAL_FORMAT>(self, options)
    }
}
impl FormatFloatLexical for f64 {
    fn format_float_lexical(self, options: &WriteFloatOptions) -> String {
        lexical::to_string_with_options::<f64, LEXICAL_FORMAT>(self, options)
    }
}

/// Formats a float for variant text output. Matches Godot's rtos_fix/num_scientific:
/// 0 → "0", nan → "nan", inf → "inf", -inf → "-inf", finite → Grisu2 short decimal.
pub fn format_float_for_variant<T: Float + FormatFloatLexical>(value: T) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() { "-inf" } else { "inf" }.to_string();
    }
    let options = WriteFloatOptions::builder()
        .trim_floats(true)
        .build()
        .unwrap();
    value.format_float_lexical(&options)
}

/// Float for variant output: use format_float_for_variant then append ".0" if result looks like an integer (Godot rule).
fn format_float_rtos_fix<T: Float + FormatFloatLexical>(value: T, compat: bool) -> String {
    if value.is_zero() {
        return "0".to_string();
    }
    if compat && value.is_infinite() && value.is_sign_negative() {
        return "inf_neg".to_string();
    }
    let s = format_float_for_variant(value);
    if s == "inf" || s == "-inf" || s == "nan" {
        return s;
    }
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        format!("{}.0", s)
    } else {
        s
    }
}

/// Escape string for variant text (multiline style: only \ and ").
fn escape_string_for_variant(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

// -----------------------------------------------------------------------------
// VariantVal (mirrors Godot Variant for parsing .tres/.tscn property values)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum ElemType {
    Identifier(String),
    Resource(Option<String>, String),
    SubResource(String),
    ExtResource(String, Option<String>, String),
}

#[derive(Clone, Debug)]
pub enum VariantVal {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),

    Vector2(RealT, RealT),
    Vector2i(i32, i32),
    Rect2((RealT, RealT), (RealT, RealT)),
    Rect2i((i32, i32), (i32, i32)),
    Vector3(RealT, RealT, RealT),
    Vector3i(i32, i32, i32),
    Transform2d((RealT, RealT), (RealT, RealT), (RealT, RealT)),
    Vector4(RealT, RealT, RealT, RealT),
    Vector4i(i32, i32, i32, i32),
    Plane((RealT, RealT, RealT), RealT),
    Quaternion(RealT, RealT, RealT, RealT),
    Aabb((RealT, RealT, RealT), (RealT, RealT, RealT)),
    Basis((RealT, RealT, RealT), (RealT, RealT, RealT), (RealT, RealT, RealT)),
    Transform3d(
        ((RealT, RealT, RealT), (RealT, RealT, RealT), (RealT, RealT, RealT)),
        (RealT, RealT, RealT),
    ),
    Projection(
        (RealT, RealT, RealT, RealT),
        (RealT, RealT, RealT, RealT),
        (RealT, RealT, RealT, RealT),
        (RealT, RealT, RealT, RealT),
    ),
    Color(f32, f32, f32, f32),
    StringName(String),
    NodePath(String),
    Rid(String),
    Object(String, IndexMap<String, Box<VariantVal>>),
    Callable,
    Signal,
    Dictionary(
        Option<(Box<ElemType>, Box<ElemType>)>,
        IndexMap<Box<VariantVal>, Box<VariantVal>>,
    ),
    Array(Option<Box<ElemType>>, Vec<Box<VariantVal>>),
    PackedByteArray(Vec<u8>),
    PackedInt32Array(Vec<i32>),
    PackedInt64Array(Vec<i64>),
    PackedFloat32Array(Vec<f32>),
    PackedFloat64Array(Vec<f64>),
    PackedStringArray(Vec<String>),
    PackedVector2Array(Vec<(RealT, RealT)>),
    PackedVector3Array(Vec<(RealT, RealT, RealT)>),
    PackedColorArray(Vec<(RealT, RealT, RealT, RealT)>),
    PackedVector4Array(Vec<(RealT, RealT, RealT, RealT)>),

    Resource(Option<String>, String),
    SubResource(String),
    ExtResource(String, Option<String>, String),
}

impl PartialEq for ElemType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ElemType::Identifier(a), ElemType::Identifier(b)) => a == b,
            (ElemType::Resource(a, b), ElemType::Resource(c, d)) => a == c && b == d,
            (ElemType::SubResource(a), ElemType::SubResource(b)) => a == b,
            (ElemType::ExtResource(a, b, c), ElemType::ExtResource(d, e, f)) => a == d && b == e && c == f,
            _ => false,
        }
    }
}

impl std::hash::Hash for ElemType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            ElemType::Identifier(s) => s.hash(state),
            ElemType::Resource(a, b) => {
                if let Some(a) = a {
                    a.hash(state);
                }
                b.hash(state);
            }
            ElemType::SubResource(s) => s.hash(state),
            ElemType::ExtResource(a, b, c) => {
                a.hash(state);
                if let Some(b) = b {
                    b.hash(state);
                }
                c.hash(state);
            }
        }
    }
}

// IndexMap key support: Hash + Eq for Dictionary. Float uses to_bits() for stability.
impl PartialEq for VariantVal {
    fn eq(&self, other: &Self) -> bool {
        use VariantVal::*;
        match (self, other) {
            (Nil, Nil) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a.to_bits() == b.to_bits(),
            (String(a), String(b)) => a == b,
            (Vector2(ax, ay), Vector2(bx, by)) => ax.to_f64().to_bits() == bx.to_f64().to_bits() && ay.to_f64().to_bits() == by.to_f64().to_bits(),
            (Vector2i(ax, ay), Vector2i(bx, by)) => (ax, ay) == (bx, by),
            (Rect2(ap, asz), Rect2(bp, bsz)) => {
                (ap.0.to_f64().to_bits(), ap.1.to_f64().to_bits(), asz.0.to_f64().to_bits(), asz.1.to_f64().to_bits())
                    == (bp.0.to_f64().to_bits(), bp.1.to_f64().to_bits(), bsz.0.to_f64().to_bits(), bsz.1.to_f64().to_bits())
            }
            (Rect2i(ap, asz), Rect2i(bp, bsz)) => (ap, asz) == (bp, bsz),
            (Vector3(ax, ay, az), Vector3(bx, by, bz)) => {
                [ax, ay, az].iter().zip([bx, by, bz].iter()).all(|(a, b)| a.to_f64().to_bits() == b.to_f64().to_bits())
            }
            (Vector3i(ax, ay, az), Vector3i(bx, by, bz)) => (ax, ay, az) == (bx, by, bz),
            (Transform2d(a0, a1, a2), Transform2d(b0, b1, b2)) => {
                (a0.0.to_f64().to_bits(), a0.1.to_f64().to_bits(), a1.0.to_f64().to_bits(), a1.1.to_f64().to_bits(), a2.0.to_f64().to_bits(), a2.1.to_f64().to_bits())
                    == (b0.0.to_f64().to_bits(), b0.1.to_f64().to_bits(), b1.0.to_f64().to_bits(), b1.1.to_f64().to_bits(), b2.0.to_f64().to_bits(), b2.1.to_f64().to_bits())
            }
            (Vector4(ax, ay, az, aw), Vector4(bx, by, bz, bw)) => {
                [ax, ay, az, aw].iter().zip([bx, by, bz, bw].iter()).all(|(a, b)| a.to_f64().to_bits() == b.to_f64().to_bits())
            }
            (Vector4i(ax, ay, az, aw), Vector4i(bx, by, bz, bw)) => (ax, ay, az, aw) == (bx, by, bz, bw),
            (Plane(an, ad), Plane(bn, bd)) => {
                (an.0.to_f64().to_bits(), an.1.to_f64().to_bits(), an.2.to_f64().to_bits(), ad.to_f64().to_bits())
                    == (bn.0.to_f64().to_bits(), bn.1.to_f64().to_bits(), bn.2.to_f64().to_bits(), bd.to_f64().to_bits())
            }
            (Quaternion(ax, ay, az, aw), Quaternion(bx, by, bz, bw)) => {
                [ax, ay, az, aw].iter().zip([bx, by, bz, bw].iter()).all(|(a, b)| a.to_f64().to_bits() == b.to_f64().to_bits())
            }
            (Aabb(ap, asz), Aabb(bp, bsz)) => {
                (ap.0.to_f64().to_bits(), ap.1.to_f64().to_bits(), ap.2.to_f64().to_bits(), asz.0.to_f64().to_bits(), asz.1.to_f64().to_bits(), asz.2.to_f64().to_bits())
                    == (bp.0.to_f64().to_bits(), bp.1.to_f64().to_bits(), bp.2.to_f64().to_bits(), bsz.0.to_f64().to_bits(), bsz.1.to_f64().to_bits(), bsz.2.to_f64().to_bits())
            }
            (Basis(a0, a1, a2), Basis(b0, b1, b2)) => {
                let abits = (a0.0.to_f64().to_bits(), a0.1.to_f64().to_bits(), a0.2.to_f64().to_bits(), a1.0.to_f64().to_bits(), a1.1.to_f64().to_bits(), a1.2.to_f64().to_bits(), a2.0.to_f64().to_bits(), a2.1.to_f64().to_bits(), a2.2.to_f64().to_bits());
                let bbits = (b0.0.to_f64().to_bits(), b0.1.to_f64().to_bits(), b0.2.to_f64().to_bits(), b1.0.to_f64().to_bits(), b1.1.to_f64().to_bits(), b1.2.to_f64().to_bits(), b2.0.to_f64().to_bits(), b2.1.to_f64().to_bits(), b2.2.to_f64().to_bits());
                abits == bbits
            }
            (Transform3d(ab, ao), Transform3d(bb, bo)) => {
                let (a0, a1, a2) = ab;
                let (b0, b1, b2) = bb;
                let abits = (a0.0.to_f64().to_bits(), a0.1.to_f64().to_bits(), a0.2.to_f64().to_bits(), a1.0.to_f64().to_bits(), a1.1.to_f64().to_bits(), a1.2.to_f64().to_bits(), a2.0.to_f64().to_bits(), a2.1.to_f64().to_bits(), a2.2.to_f64().to_bits(), ao.0.to_f64().to_bits(), ao.1.to_f64().to_bits(), ao.2.to_f64().to_bits());
                let bbits = (b0.0.to_f64().to_bits(), b0.1.to_f64().to_bits(), b0.2.to_f64().to_bits(), b1.0.to_f64().to_bits(), b1.1.to_f64().to_bits(), b1.2.to_f64().to_bits(), b2.0.to_f64().to_bits(), b2.1.to_f64().to_bits(), b2.2.to_f64().to_bits(), bo.0.to_f64().to_bits(), bo.1.to_f64().to_bits(), bo.2.to_f64().to_bits());
                abits == bbits
            }
            (Projection(a0, a1, a2, a3), Projection(b0, b1, b2, b3)) => {
                let abits = [
                    a0.0.to_f64().to_bits(), a0.1.to_f64().to_bits(), a0.2.to_f64().to_bits(), a0.3.to_f64().to_bits(),
                    a1.0.to_f64().to_bits(), a1.1.to_f64().to_bits(), a1.2.to_f64().to_bits(), a1.3.to_f64().to_bits(),
                    a2.0.to_f64().to_bits(), a2.1.to_f64().to_bits(), a2.2.to_f64().to_bits(), a2.3.to_f64().to_bits(),
                    a3.0.to_f64().to_bits(), a3.1.to_f64().to_bits(), a3.2.to_f64().to_bits(), a3.3.to_f64().to_bits(),
                ];
                let bbits = [
                    b0.0.to_f64().to_bits(), b0.1.to_f64().to_bits(), b0.2.to_f64().to_bits(), b0.3.to_f64().to_bits(),
                    b1.0.to_f64().to_bits(), b1.1.to_f64().to_bits(), b1.2.to_f64().to_bits(), b1.3.to_f64().to_bits(),
                    b2.0.to_f64().to_bits(), b2.1.to_f64().to_bits(), b2.2.to_f64().to_bits(), b2.3.to_f64().to_bits(),
                    b3.0.to_f64().to_bits(), b3.1.to_f64().to_bits(), b3.2.to_f64().to_bits(), b3.3.to_f64().to_bits(),
                ];
                abits.iter().zip(bbits.iter()).all(|(a, b)| a == b)
            }
            (Color(ar, ag, ab, aa), Color(br, bg, bb, ba)) => {
                [*ar, *ag, *ab, *aa].iter().zip([*br, *bg, *bb, *ba].iter()).all(|(x, y)| x.to_bits() == y.to_bits())
            }
            (StringName(a), StringName(b)) => a == b,
            (NodePath(a), NodePath(b)) => a == b,
            (Rid(a), Rid(b)) => a == b,
            (Object(ta, pa), Object(tb, pb)) => ta == tb && pa == pb,
            (Callable, Callable) | (Signal, Signal) => true,
            (Dictionary(type_a, ma), Dictionary(type_b, mb)) => type_a == type_b && ma == mb,
            (Array(type_a, aa), Array(type_b, ab)) => type_a == type_b && aa == ab,
            (PackedByteArray(a), PackedByteArray(b)) => a == b,
            (PackedInt32Array(a), PackedInt32Array(b)) => a == b,
            (PackedInt64Array(a), PackedInt64Array(b)) => a == b,
            (PackedFloat32Array(a), PackedFloat32Array(b)) => a.iter().zip(b.iter()).all(|(x, y)| x.to_bits() == y.to_bits()),
            (PackedFloat64Array(a), PackedFloat64Array(b)) => a.iter().zip(b.iter()).all(|(x, y)| x.to_bits() == y.to_bits()),
            (PackedStringArray(a), PackedStringArray(b)) => a == b,
            (PackedVector2Array(a), PackedVector2Array(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(p, q)| p.0.to_f64().to_bits() == q.0.to_f64().to_bits() && p.1.to_f64().to_bits() == q.1.to_f64().to_bits()),
            (PackedVector3Array(a), PackedVector3Array(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(p, q)| {
                [p.0.to_f64(), p.1.to_f64(), p.2.to_f64()].iter().zip([q.0.to_f64(), q.1.to_f64(), q.2.to_f64()].iter()).all(|(x, y)| x.to_bits() == y.to_bits())
            }),
            (PackedVector4Array(a), PackedVector4Array(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(p, q)| {
                [p.0.to_f64(), p.1.to_f64(), p.2.to_f64(), p.3.to_f64()].iter().zip([q.0.to_f64(), q.1.to_f64(), q.2.to_f64(), q.3.to_f64()].iter()).all(|(x, y)| x.to_bits() == y.to_bits())
            }),
            (PackedColorArray(a), PackedColorArray(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(p, q)| {
                [p.0.to_f64(), p.1.to_f64(), p.2.to_f64(), p.3.to_f64()].iter().zip([q.0.to_f64(), q.1.to_f64(), q.2.to_f64(), q.3.to_f64()].iter()).all(|(x, y)| x.to_bits() == y.to_bits())
            }),
            (Resource(ua, pa), Resource(ub, pb)) => (ua, pa) == (ub, pb),
            (SubResource(a), SubResource(b)) => a == b,
            (ExtResource(ia, ua, pa), ExtResource(ib, ub, pb)) => (ia, ua, pa) == (ib, ub, pb),
            _ => false,
        }
    }
}

impl Eq for VariantVal {}

impl std::hash::Hash for VariantVal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            VariantVal::Nil => {}
            VariantVal::Bool(b) => b.hash(state),
            VariantVal::Int(i) => i.hash(state),
            VariantVal::Float(f) => f.to_bits().hash(state),
            VariantVal::String(s) => s.hash(state),
            VariantVal::Vector2(x, y) => {
                x.to_f64().to_bits().hash(state);
                y.to_f64().to_bits().hash(state);
            }
            VariantVal::Vector2i(x, y) => (x, y).hash(state),
            VariantVal::Rect2((x0, y0), (x1, y1)) => {
                (x0.to_f64().to_bits(), y0.to_f64().to_bits(), x1.to_f64().to_bits(), y1.to_f64().to_bits()).hash(state);
            }
            VariantVal::Rect2i(a, b) => (a, b).hash(state),
            VariantVal::Vector3(x, y, z) => (x.to_f64().to_bits(), y.to_f64().to_bits(), z.to_f64().to_bits()).hash(state),
            VariantVal::Vector3i(x, y, z) => (x, y, z).hash(state),
            VariantVal::StringName(s) => s.hash(state),
            VariantVal::NodePath(s) => s.hash(state),
            VariantVal::Rid(s) => s.hash(state),
            VariantVal::Object(t, props) => {
                t.hash(state);
                for (k, v) in props {
                    k.hash(state);
                    v.hash(state);
                }
            }
            VariantVal::Callable | VariantVal::Signal => {}
            VariantVal::Dictionary(_, map) => {
                for (k, v) in map {
                    k.hash(state);
                    v.hash(state);
                }
            }
            VariantVal::Array(_, arr) => {
                for v in arr {
                    v.hash(state);
                }
            }
            VariantVal::PackedByteArray(b) => b.hash(state),
            VariantVal::PackedInt32Array(a) => a.hash(state),
            VariantVal::PackedInt64Array(a) => a.hash(state),
            VariantVal::PackedFloat32Array(a) => a.iter().for_each(|f| f.to_bits().hash(state)),
            VariantVal::PackedFloat64Array(a) => a.iter().for_each(|f| f.to_bits().hash(state)),
            VariantVal::PackedStringArray(a) => a.hash(state),
            VariantVal::PackedVector2Array(a) => a.iter().for_each(|(x, y)| {
                x.to_f64().to_bits().hash(state);
                y.to_f64().to_bits().hash(state);
            }),
            VariantVal::PackedVector3Array(a) => a.iter().for_each(|(x, y, z)| {
                x.to_f64().to_bits().hash(state);
                y.to_f64().to_bits().hash(state);
                z.to_f64().to_bits().hash(state);
            }),
            VariantVal::PackedVector4Array(a) => a.iter().for_each(|(x, y, z, w)| {
                x.to_f64().to_bits().hash(state);
                y.to_f64().to_bits().hash(state);
                z.to_f64().to_bits().hash(state);
                w.to_f64().to_bits().hash(state);
            }),
            VariantVal::PackedColorArray(a) => a.iter().for_each(|(r, g, b, a)| {
                r.to_f64().to_bits().hash(state);
                g.to_f64().to_bits().hash(state);
                b.to_f64().to_bits().hash(state);
                a.to_f64().to_bits().hash(state);
            }),
            VariantVal::Resource(u, p) => (u, p).hash(state),
            VariantVal::SubResource(s) => s.hash(state),
            VariantVal::ExtResource(i, u, p) => (i, u, p).hash(state),
            VariantVal::Transform2d(a0, a1, a2) => {
                (a0.0.to_f64().to_bits(), a0.1.to_f64().to_bits(), a1.0.to_f64().to_bits(), a1.1.to_f64().to_bits(), a2.0.to_f64().to_bits(), a2.1.to_f64().to_bits()).hash(state);
            }
            VariantVal::Vector4(x, y, z, w) => (x.to_f64().to_bits(), y.to_f64().to_bits(), z.to_f64().to_bits(), w.to_f64().to_bits()).hash(state),
            VariantVal::Vector4i(x, y, z, w) => (x, y, z, w).hash(state),
            VariantVal::Plane(n, d) => (n.0.to_f64().to_bits(), n.1.to_f64().to_bits(), n.2.to_f64().to_bits(), d.to_f64().to_bits()).hash(state),
            VariantVal::Quaternion(x, y, z, w) => (x.to_f64().to_bits(), y.to_f64().to_bits(), z.to_f64().to_bits(), w.to_f64().to_bits()).hash(state),
            VariantVal::Aabb(p, s) => (p.0.to_f64().to_bits(), p.1.to_f64().to_bits(), p.2.to_f64().to_bits(), s.0.to_f64().to_bits(), s.1.to_f64().to_bits(), s.2.to_f64().to_bits()).hash(state),
            VariantVal::Basis(a0, a1, a2) => {
                (a0.0.to_f64().to_bits(), a0.1.to_f64().to_bits(), a0.2.to_f64().to_bits(), a1.0.to_f64().to_bits(), a1.1.to_f64().to_bits(), a1.2.to_f64().to_bits(), a2.0.to_f64().to_bits(), a2.1.to_f64().to_bits(), a2.2.to_f64().to_bits()).hash(state);
            }
            VariantVal::Transform3d(b, o) => {
                let (b0, b1, b2) = b;
                (b0.0.to_f64().to_bits(), b0.1.to_f64().to_bits(), b0.2.to_f64().to_bits(), b1.0.to_f64().to_bits(), b1.1.to_f64().to_bits(), b1.2.to_f64().to_bits(), b2.0.to_f64().to_bits(), b2.1.to_f64().to_bits(), b2.2.to_f64().to_bits(), o.0.to_f64().to_bits(), o.1.to_f64().to_bits(), o.2.to_f64().to_bits()).hash(state);
            }
            VariantVal::Projection(a0, a1, a2, a3) => {
                // flatten all fields to bits and hash in order, since tuples of 16 u64 do not implement Hash
                let fields = [
                    a0.0.to_f64().to_bits(), a0.1.to_f64().to_bits(), a0.2.to_f64().to_bits(), a0.3.to_f64().to_bits(),
                    a1.0.to_f64().to_bits(), a1.1.to_f64().to_bits(), a1.2.to_f64().to_bits(), a1.3.to_f64().to_bits(),
                    a2.0.to_f64().to_bits(), a2.1.to_f64().to_bits(), a2.2.to_f64().to_bits(), a2.3.to_f64().to_bits(),
                    a3.0.to_f64().to_bits(), a3.1.to_f64().to_bits(), a3.2.to_f64().to_bits(), a3.3.to_f64().to_bits(),
                ];
                for f in fields {
                    f.hash(state);
                }
            }
            VariantVal::Color(r, g, b, a) => (r.to_bits(), g.to_bits(), b.to_bits(), a.to_bits()).hash(state),
        }
    }
}

// -----------------------------------------------------------------------------
// Lexer (tokenizer) — mirrors Godot's get_token
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Token {
    CurlyOpen,
    CurlyClose,
    BracketOpen,
    BracketClose,
    ParenOpen,
    ParenClose,
    Colon,
    Comma,
    Identifier(String),
    Str(String),
    StringName(String),
    Number { int: Option<i64>, float: f64 }, // if int is Some, value is integer; else float
    Color { r: f32, g: f32, b: f32, a: f32 },
    Eof,
}

fn is_ascii_alpha(c: char) -> bool {
    c.is_ascii_alphabetic()
}
fn is_underscore(c: char) -> bool {
    c == '_'
}
fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}
fn is_hex_digit(c: char) -> bool {
    c.is_ascii_hexdigit()
}

fn stor_fix(s: &str) -> Option<f64> {
    match s {
        "inf" => Some(f64::INFINITY),
        "-inf" | "inf_neg" => Some(f64::NEG_INFINITY),
        "nan" => Some(f64::NAN),
        _ => None,
    }
}

/// Parse #hex color to (r, g, b, a) in 0.0..=1.0. Matches Godot Color::html().
/// Supports: #rgb (3), #rgba (4), #rrggbb (6), #rrggbbaa (8).
fn parse_color_hex(hex: &str) -> Result<(f32, f32, f32, f32), VariantParseError> {
    fn parse_col4(s: &str, ofs: usize) -> Result<f32, VariantParseError> {
        let c = s.chars().nth(ofs).ok_or_else(|| VariantParseError("Invalid color code".into()))?;
        let v = match c {
            '0'..='9' => c as u32 - '0' as u32,
            'a'..='f' => c as u32 - 'a' as u32 + 10,
            'A'..='F' => c as u32 - 'A' as u32 + 10,
            _ => return Err(VariantParseError("Invalid color code".into())),
        };
        Ok(v as f32)
    }
    fn parse_col8(s: &str, ofs: usize) -> Result<f32, VariantParseError> {
        let hi = parse_col4(s, ofs)? as u32;
        let lo = parse_col4(s, ofs + 1)? as u32;
        Ok((hi * 16 + lo) as f32)
    }

    let n = hex.len();
    let (r, g, b, a) = if n == 3 {
        let r = parse_col4(hex, 0)? / 15.0;
        let g = parse_col4(hex, 1)? / 15.0;
        let b = parse_col4(hex, 2)? / 15.0;
        (r, g, b, 1.0)
    } else if n == 4 {
        let r = parse_col4(hex, 0)? / 15.0;
        let g = parse_col4(hex, 1)? / 15.0;
        let b = parse_col4(hex, 2)? / 15.0;
        let a = parse_col4(hex, 3)? / 15.0;
        (r, g, b, a)
    } else if n == 6 {
        let r = parse_col8(hex, 0)? / 255.0;
        let g = parse_col8(hex, 2)? / 255.0;
        let b = parse_col8(hex, 4)? / 255.0;
        (r, g, b, 1.0)
    } else if n == 8 {
        let r = parse_col8(hex, 0)? / 255.0;
        let g = parse_col8(hex, 2)? / 255.0;
        let b = parse_col8(hex, 4)? / 255.0;
        let a = parse_col8(hex, 6)? / 255.0;
        (r, g, b, a)
    } else {
        return Err(VariantParseError(format!("Invalid color code: expected 3, 4, 6, or 8 hex digits, got {}", n)));
    };
    if r < 0.0 || g < 0.0 || b < 0.0 || a < 0.0 {
        return Err(VariantParseError("Invalid color code".into()));
    }
    Ok((r, g, b, a))
}

struct Lexer<'a> {
    chars: std::str::Chars<'a>,
    saved: Option<char>,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Lexer {
            chars: s.chars(),
            saved: None,
        }
    }

    fn get_char(&mut self) -> Option<char> {
        if let Some(c) = self.saved.take() {
            return Some(c);
        }
        self.chars.next()
    }

    fn put_back(&mut self, c: char) {
        debug_assert!(self.saved.is_none());
        self.saved = Some(c);
    }

    fn next_token(&mut self) -> Result<Token, VariantParseError> {
        loop {
            let cchar = self.get_char();
            let cchar = match cchar {
                None => return Ok(Token::Eof),
                Some('\n') => continue,
                Some(c) if c <= ' ' => continue,
                Some(c) => c,
            };

            return match cchar {
                '{' => Ok(Token::CurlyOpen),
                '}' => Ok(Token::CurlyClose),
                '[' => Ok(Token::BracketOpen),
                ']' => Ok(Token::BracketClose),
                '(' => Ok(Token::ParenOpen),
                ')' => Ok(Token::ParenClose),
                ':' => Ok(Token::Colon),
                ',' => Ok(Token::Comma),
                ';' => {
                    while let Some(ch) = self.get_char() {
                        if ch == '\n' {
                            break;
                        }
                    }
                    continue;
                }
                '#' => {
                    let mut hex = String::new();
                    loop {
                        match self.get_char() {
                            Some(ch) if is_hex_digit(ch) => hex.push(ch),
                            other => {
                                if let Some(c) = other {
                                    self.put_back(c);
                                }
                                break;
                            }
                        }
                    }
                    // Match Godot Color::html(): #rgb (3), #rgba (4), #rrggbb (6), #rrggbbaa (8)
                    let (r, g, b, a) = parse_color_hex(&hex)?;
                    Ok(Token::Color { r, g, b, a })
                }
                '&' => {
                    if self.get_char() != Some('"') {
                        return Err(VariantParseError("Expected '\"' after '&'".into()));
                    }
                    let s = self.parse_string()?;
                    return Ok(Token::StringName(s));
                }
                '"' => {
                    let s = self.parse_string()?;
                    Ok(Token::Str(s))
                }
                '-' => {
                    let next = self.get_char();
                    match next {
                        Some(c) if is_digit(c) => {
                            self.put_back(c);
                            self.put_back('-');
                            Ok(self.parse_number()?)
                        }
                        Some(c) if is_ascii_alpha(c) || is_underscore(c) => {
                            // Identifier like -inf, inf_neg (Godot allows minus-prefix for these)
                            let mut token_text = String::from("-");
                            let mut cur = c;
                            let mut first = true;
                            loop {
                                if is_ascii_alpha(cur) || is_underscore(cur) || (!first && is_digit(cur)) {
                                    token_text.push(cur);
                                    first = false;
                                    cur = match self.get_char() {
                                        Some(c) => c,
                                        None => break,
                                    };
                                } else {
                                    self.put_back(cur);
                                    break;
                                }
                            }
                            Ok(Token::Identifier(token_text))
                        }
                        other => {
                            if let Some(c) = other {
                                self.put_back(c);
                            }
                            self.put_back('-');
                            Err(VariantParseError("Unexpected character '-'".into()))
                        }
                    }
                }
                c if is_digit(c) => {
                    self.put_back(c);
                    Ok(self.parse_number()?)
                }
                c if is_ascii_alpha(c) || is_underscore(c) => {
                    let mut token_text = String::new();
                    let mut cur = c;
                    let mut first = true;
                    loop {
                        if is_ascii_alpha(cur) || is_underscore(cur) || (!first && is_digit(cur)) {
                            token_text.push(cur);
                            first = false;
                            cur = match self.get_char() {
                                Some(c) => c,
                                None => break,
                            };
                        } else {
                            self.put_back(cur);
                            break;
                        }
                    }
                    Ok(Token::Identifier(token_text))
                }
                _ => Err(VariantParseError(format!("Unexpected character '{}'", cchar))),
            };
        }
    }

    fn parse_string(&mut self) -> Result<String, VariantParseError> {
        let mut s = String::new();
        loop {
            let ch = self.get_char().ok_or_else(|| VariantParseError("Unterminated string".into()))?;
            if ch == '"' {
                break;
            }
            if ch == '\\' {
                let next = self.get_char().ok_or_else(|| VariantParseError("Unterminated string".into()))?;
                let decoded = match next {
                    'b' => '\u{8}',
                    't' => '\t',
                    'n' => '\n',
                    'f' => '\u{c}',
                    'r' => '\r',
                    'u' => self.parse_hex_escape(4)?,
                    'U' => self.parse_hex_escape(6)?,
                    c => c,
                };
                s.push(decoded);
            } else {
                s.push(ch);
            }
        }
        Ok(s)
    }

    fn parse_hex_escape(&mut self, len: usize) -> Result<char, VariantParseError> {
        let mut v: u32 = 0;
        for _ in 0..len {
            let c = self.get_char().ok_or_else(|| VariantParseError("Unterminated string".into()))?;
            let digit = match c {
                '0'..='9' => c as u32 - '0' as u32,
                'a'..='f' => c as u32 - 'a' as u32 + 10,
                'A'..='F' => c as u32 - 'A' as u32 + 10,
                _ => return Err(VariantParseError("Malformed hex constant in string".into())),
            };
            v = (v << 4) | digit;
        }
        char::from_u32(v).ok_or_else(|| VariantParseError("Invalid Unicode scalar in string".into()))
    }

    fn parse_number(&mut self) -> Result<Token, VariantParseError> {
        let mut token_text = String::new();
        let mut neg = false;
        let first = self.get_char();
        if first == Some('-') {
            neg = true;
            token_text.push('-');
        } else if let Some(c) = first {
            token_text.push(c);
        }
        let mut reading_int = true;
        let mut is_float = false;
        loop {
            let c = self.get_char();
            let c = match c {
                Some(c) => c,
                None => break,
            };
            match (reading_int, c) {
                (true, c) if is_digit(c) => token_text.push(c),
                (true, '.') => {
                    token_text.push(c);
                    reading_int = false;
                    is_float = true;
                }
                (true, 'e' | 'E') => {
                    token_text.push(c);
                    reading_int = false;
                    is_float = true;
                }
                (false, c) if is_digit(c) => token_text.push(c),
                (false, 'e' | 'E') => {
                    token_text.push(c);
                    is_float = true;
                }
                (false, '+' | '-') => token_text.push(c),
                _ => {
                    self.put_back(c);
                    break;
                }
            }
        }
        if is_float {
            let f: f64 = token_text.parse().map_err(|_| VariantParseError("Invalid number".into()))?;
            Ok(Token::Number { int: None, float: f })
        } else {
            let i: i64 = token_text.parse().map_err(|_| VariantParseError("Invalid integer".into()))?;
            Ok(Token::Number {
                int: Some(i),
                float: i as f64,
            })
        }
    }
}

// -----------------------------------------------------------------------------
// Parser — recursive descent, mirrors parse_value / _parse_dictionary / _parse_array
// -----------------------------------------------------------------------------

struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Option<Token>,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            lexer: Lexer::new(s),
            current: None,
        }
    }

    fn get_token(&mut self) -> Result<&Token, VariantParseError> {
        if self.current.is_none() {
            self.current = Some(self.lexer.next_token()?);
        }
        Ok(self.current.as_ref().unwrap())
    }

    fn advance(&mut self) -> Result<Token, VariantParseError> {
        let tok = if let Some(t) = self.current.take() {
            t
        } else {
            self.lexer.next_token()?
        };
        self.current = None;
        Ok(tok)
    }

    fn expect(&mut self, want: &str) -> Result<Token, VariantParseError> {
        let t = self.advance()?;
        match &t {
            Token::ParenOpen if want == "(" => {}
            Token::ParenClose if want == ")" => {}
            Token::CurlyOpen if want == "{" => {}
            Token::CurlyClose if want == "}" => {}
            Token::BracketOpen if want == "[" => {}
            Token::BracketClose if want == "]" => {}
            Token::Colon if want == ":" => {}
            Token::Comma if want == "," => {}
            _ => return Err(VariantParseError(format!("Expected '{}'", want))),
        }
        Ok(t)
    }

    fn parse_value(&mut self, token: Token) -> Result<VariantVal, VariantParseError> {
        match token {
            Token::CurlyOpen => self.parse_dictionary(),
            Token::BracketOpen => self.parse_array(),
            Token::Identifier(id) => self.parse_identifier(&id),
            Token::Number { int, float } => {
                if let Some(i) = int {
                    Ok(VariantVal::Int(i))
                } else {
                    Ok(VariantVal::Float(float))
                }
            }
            Token::Str(s) => Ok(VariantVal::String(s)),
            Token::StringName(s) => Ok(VariantVal::StringName(s)),
            Token::Color { r, g, b, a } => Ok(VariantVal::Color(
                r as f32,
                g as f32,
                b as f32,
                a as f32,
            )),
            Token::Eof => Err(VariantParseError("Unexpected EOF".into())),
            _ => Err(VariantParseError(format!("Expected value, got token"))),
        }
    }

    fn parse_dictionary_body(&mut self) -> Result<IndexMap<Box<VariantVal>, Box<VariantVal>>, VariantParseError> {
        let mut map = IndexMap::new();
        loop {
            let tok = self.advance()?;
            if matches!(tok, Token::CurlyClose) {
                break;
            }
            if matches!(tok, Token::Eof) {
                return Err(VariantParseError("Unexpected EOF while parsing dictionary".into()));
            }
            let key = self.parse_value(tok)?;
            self.expect(":")?;
            let val_tok = self.advance()?;
            if matches!(val_tok, Token::Eof) {
                return Err(VariantParseError("Unexpected EOF while parsing dictionary".into()));
            }
            let val = self.parse_value(val_tok)?;
            map.insert(Box::new(key), Box::new(val));
            let next = self.get_token()?;
            if matches!(next, Token::CurlyClose) {
                continue;
            }
            if !matches!(next, Token::Comma) {
                return Err(VariantParseError("Expected '}' or ','".into()));
            }
            self.advance()?; // consume comma
        }
        Ok(map)
    }

    fn parse_dictionary(&mut self) -> Result<VariantVal, VariantParseError> {
        let map = self.parse_dictionary_body()?;
        Ok(VariantVal::Dictionary(None, map))
    }

    fn parse_array_body(&mut self) -> Result<Vec<Box<VariantVal>>, VariantParseError> {
        let mut arr = Vec::new();
        loop {
            if matches!(self.get_token()?, Token::BracketClose) {
                break;
            }
            let tok = self.advance()?;
            if matches!(tok, Token::Eof) {
                return Err(VariantParseError("Unexpected EOF while parsing array".into()));
            }
            let val = self.parse_value(tok)?;
            arr.push(Box::new(val));
            let next = self.get_token()?;
            if matches!(next, Token::BracketClose) {
                break;
            }
            if !matches!(next, Token::Comma) {
                return Err(VariantParseError("Expected ','".into()));
            }
            self.advance()?;
        }
        Ok(arr)
    }

    fn parse_array(&mut self) -> Result<VariantVal, VariantParseError> {
        let arr = self.parse_array_body()?;
        // expect closing bracket
        self.expect("]")?;
        Ok(VariantVal::Array(None, arr))
    }

    fn parse_construct_real(&mut self, count: usize) -> Result<Vec<f64>, VariantParseError> {
        self.expect("(")?;
        let mut args = Vec::new();
        let mut first = true;
        loop {
            if !first {
                let t = self.advance()?;
                if matches!(t, Token::ParenClose) {
                    break;
                }
                if !matches!(t, Token::Comma) {
                    return Err(VariantParseError("Expected ',' or ')' in constructor".into()));
                }
            }
            let t = self.advance()?;
            if first && matches!(t, Token::ParenClose) {
                break;
            }
            let f = match &t {
                Token::Number { int, float } => *float,
                Token::Identifier(id) => {
                    stor_fix(id).ok_or_else(|| VariantParseError("Expected float in constructor".into()))?
                }
                _ => return Err(VariantParseError("Expected float in constructor".into())),
            };
            args.push(f);
            first = false;
        }
        if args.len() != count {
            return Err(VariantParseError(format!(
                "Expected {} arguments for constructor, got {}",
                count,
                args.len()
            )));
        }
        Ok(args)
    }

    fn parse_construct_int(&mut self, count: usize) -> Result<Vec<i32>, VariantParseError> {
        let reals = self.parse_construct_real(count)?;
        reals
            .into_iter()
            .map(|f| i32::try_from(f as i64).map_err(|_| VariantParseError("Integer overflow in constructor".into())))
            .collect()
    }

    fn parse_identifier(&mut self, id: &str) -> Result<VariantVal, VariantParseError> {
        match id {
            "true" => return Ok(VariantVal::Bool(true)),
            "false" => return Ok(VariantVal::Bool(false)),
            "null" | "nil" => return Ok(VariantVal::Nil),
            "inf" => return Ok(VariantVal::Float(f64::INFINITY)),
            "-inf" | "inf_neg" => return Ok(VariantVal::Float(f64::NEG_INFINITY)),
            "nan" => return Ok(VariantVal::Float(f64::NAN)),
            _ => {}
        }

        if id == "Vector2" {
            let a = self.parse_construct_real(2)?;
            return Ok(VariantVal::Vector2(RealT::F64(a[0]), RealT::F64(a[1])));
        }
        if id == "Vector2i" {
            let a = self.parse_construct_int(2)?;
            return Ok(VariantVal::Vector2i(a[0], a[1]));
        }
        if id == "Rect2" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Rect2(
                (RealT::F64(a[0]), RealT::F64(a[1])),
                (RealT::F64(a[2]), RealT::F64(a[3])),
            ));
        }
        if id == "Rect2i" {
            let a = self.parse_construct_int(4)?;
            return Ok(VariantVal::Rect2i((a[0], a[1]), (a[2], a[3])));
        }
        if id == "Vector3" {
            let a = self.parse_construct_real(3)?;
            return Ok(VariantVal::Vector3(RealT::F64(a[0]), RealT::F64(a[1]), RealT::F64(a[2])));
        }
        if id == "Vector3i" {
            let a = self.parse_construct_int(3)?;
            return Ok(VariantVal::Vector3i(a[0], a[1], a[2]));
        }
        if id == "Vector4" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Vector4(
                RealT::F64(a[0]),
                RealT::F64(a[1]),
                RealT::F64(a[2]),
                RealT::F64(a[3]),
            ));
        }
        if id == "Vector4i" {
            let a = self.parse_construct_int(4)?;
            return Ok(VariantVal::Vector4i(a[0], a[1], a[2], a[3]));
        }
        if id == "Transform2D" || id == "Matrix32" {
            let a = self.parse_construct_real(6)?;
            return Ok(VariantVal::Transform2d(
                (RealT::F64(a[0]), RealT::F64(a[1])),
                (RealT::F64(a[2]), RealT::F64(a[3])),
                (RealT::F64(a[4]), RealT::F64(a[5])),
            ));
        }
        if id == "Plane" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Plane(
                (RealT::F64(a[0]), RealT::F64(a[1]), RealT::F64(a[2])),
                RealT::F64(a[3]),
            ));
        }
        if id == "Quaternion" || id == "Quat" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Quaternion(
                RealT::F64(a[0]),
                RealT::F64(a[1]),
                RealT::F64(a[2]),
                RealT::F64(a[3]),
            ));
        }
        if id == "AABB" || id == "Rect3" {
            let a = self.parse_construct_real(6)?;
            return Ok(VariantVal::Aabb(
                (RealT::F64(a[0]), RealT::F64(a[1]), RealT::F64(a[2])),
                (RealT::F64(a[3]), RealT::F64(a[4]), RealT::F64(a[5])),
            ));
        }
        if id == "Basis" || id == "Matrix3" {
            let a = self.parse_construct_real(9)?;
            return Ok(VariantVal::Basis(
                (RealT::F64(a[0]), RealT::F64(a[1]), RealT::F64(a[2])),
                (RealT::F64(a[3]), RealT::F64(a[4]), RealT::F64(a[5])),
                (RealT::F64(a[6]), RealT::F64(a[7]), RealT::F64(a[8])),
            ));
        }
        if id == "Transform3D" || id == "Transform" {
            let a = self.parse_construct_real(12)?;
            return Ok(VariantVal::Transform3d(
                (
                    (RealT::F64(a[0]), RealT::F64(a[1]), RealT::F64(a[2])),
                    (RealT::F64(a[3]), RealT::F64(a[4]), RealT::F64(a[5])),
                    (RealT::F64(a[6]), RealT::F64(a[7]), RealT::F64(a[8])),
                ),
                (RealT::F64(a[9]), RealT::F64(a[10]), RealT::F64(a[11])),
            ));
        }
        if id == "Projection" {
            let a = self.parse_construct_real(16)?;
            return Ok(VariantVal::Projection(
                (
                    RealT::F64(a[0]),
                    RealT::F64(a[1]),
                    RealT::F64(a[2]),
                    RealT::F64(a[3]),
                ),
                (
                    RealT::F64(a[4]),
                    RealT::F64(a[5]),
                    RealT::F64(a[6]),
                    RealT::F64(a[7]),
                ),
                (
                    RealT::F64(a[8]),
                    RealT::F64(a[9]),
                    RealT::F64(a[10]),
                    RealT::F64(a[11]),
                ),
                (
                    RealT::F64(a[12]),
                    RealT::F64(a[13]),
                    RealT::F64(a[14]),
                    RealT::F64(a[15]),
                ),
            ));
        }
        if id == "Color" {
            let a = self.parse_construct_real(4)?;
            return Ok(VariantVal::Color(
                a[0] as f32,
                a[1] as f32,
                a[2] as f32,
                a[3] as f32,
            ));
        }
        if id == "NodePath" {
            self.expect("(")?;
            let t = self.advance()?;
            let s = match &t {
                Token::Str(ss) => ss.clone(),
                _ => return Err(VariantParseError("Expected string as argument for NodePath()".into())),
            };
            self.expect(")")?;
            return Ok(VariantVal::NodePath(s));
        }
        if id == "RID" {
            self.expect("(")?;
            let t = self.advance()?;
            let s = match &t {
                Token::ParenClose => String::new(),
                Token::Number { int, .. } => int.unwrap_or(0).to_string(),
                Token::Identifier(x) => x.clone(),
                _ => return Err(VariantParseError("Expected number as argument or ')'".into())),
            };
            if !matches!(&t, Token::ParenClose) {
                self.expect(")")?;
            }
            return Ok(VariantVal::Rid(s));
        }
        if id == "Signal" {
            self.expect("(")?;
            self.expect(")")?;
            return Ok(VariantVal::Signal);
        }
        if id == "Callable" {
            self.expect("(")?;
            self.expect(")")?;
            return Ok(VariantVal::Callable);
        }
        if id == "Object" {
            return self.parse_object();
        }
        if id == "Resource" || id == "SubResource" || id == "ExtResource" {
            return self.parse_resource(id);
        }
        if id == "Dictionary" {
            return self.parse_typed_dictionary();
        }
        if id == "Array" {
            return self.parse_typed_array();
        }
        if id == "PackedByteArray" || id == "PoolByteArray" || id == "ByteArray" {
            return self.parse_packed_byte_array();
        }
        if id == "PackedInt32Array" || id == "PackedIntArray" || id == "PoolIntArray" || id == "IntArray" {
            let a = self.parse_construct_int_variadic()?;
            return Ok(VariantVal::PackedInt32Array(a));
        }
        if id == "PackedInt64Array" {
            self.expect("(")?;
            let mut args = Vec::new();
            let mut first = true;
            loop {
                if !first {
                    let t = self.advance()?;
                    if matches!(t, Token::ParenClose) {
                        break;
                    }
                    if !matches!(t, Token::Comma) {
                        return Err(VariantParseError("Expected ',' or ')'".into()));
                    }
                }
                let t = self.advance()?;
                if first && matches!(t, Token::ParenClose) {
                    break;
                }
                let i = match &t {
                    Token::Number { int, float } => int.unwrap_or(*float as i64),
                    Token::Identifier(x) => stor_fix(x).map(|f| f as i64).unwrap_or(0),
                    _ => return Err(VariantParseError("Expected number".into())),
                };
                args.push(i);
                first = false;
            }
            return Ok(VariantVal::PackedInt64Array(args));
        }
        if id == "PackedFloat32Array" || id == "PackedRealArray" || id == "PoolRealArray" || id == "FloatArray" {
            let a = self.parse_construct_real_variadic()?;
            return Ok(VariantVal::PackedFloat32Array(a.into_iter().map(|f| f as f32).collect()));
        }
        if id == "PackedFloat64Array" {
            let a = self.parse_construct_real_variadic()?;
            return Ok(VariantVal::PackedFloat64Array(a));
        }
        if id == "PackedStringArray" || id == "PoolStringArray" || id == "StringArray" {
            self.expect("(")?;
            let mut cs = Vec::new();
            let mut first = true;
            loop {
                if !first {
                    let t = self.advance()?;
                    if matches!(t, Token::ParenClose) {
                        break;
                    }
                    if !matches!(t, Token::Comma) {
                        return Err(VariantParseError("Expected ',' or ')'".into()));
                    }
                }
                let t = self.advance()?;
                if first && matches!(t, Token::ParenClose) {
                    break;
                }
                let s = match &t {
                    Token::Str(ss) => ss.clone(),
                    _ => return Err(VariantParseError("Expected string".into())),
                };
                cs.push(s);
                first = false;
            }
            return Ok(VariantVal::PackedStringArray(cs));
        }
        if id == "PackedVector2Array" || id == "PoolVector2Array" || id == "Vector2Array" {
            let a = self.parse_construct_real_variadic()?;
            if a.len() % 2 != 0 {
                return Err(VariantParseError("PackedVector2Array requires even number of components".into()));
            }
            let pairs: Vec<_> = a.chunks(2).map(|c| (RealT::F64(c[0]), RealT::F64(c[1]))).collect();
            return Ok(VariantVal::PackedVector2Array(pairs));
        }
        if id == "PackedVector3Array" || id == "PoolVector3Array" || id == "Vector3Array" {
            let a = self.parse_construct_real_variadic()?;
            if a.len() % 3 != 0 {
                return Err(VariantParseError("PackedVector3Array requires multiple of 3 components".into()));
            }
            let triples: Vec<_> = a
                .chunks(3)
                .map(|c| (RealT::F64(c[0]), RealT::F64(c[1]), RealT::F64(c[2])))
                .collect();
            return Ok(VariantVal::PackedVector3Array(triples));
        }
        if id == "PackedVector4Array" || id == "PoolVector4Array" || id == "Vector4Array" {
            let a = self.parse_construct_real_variadic()?;
            if a.len() % 4 != 0 {
                return Err(VariantParseError("PackedVector4Array requires multiple of 4 components".into()));
            }
            let quads: Vec<_> = a
                .chunks(4)
                .map(|c| (RealT::F64(c[0]), RealT::F64(c[1]), RealT::F64(c[2]), RealT::F64(c[3])))
                .collect();
            return Ok(VariantVal::PackedVector4Array(quads));
        }
        if id == "PackedColorArray" || id == "PoolColorArray" || id == "ColorArray" {
            let a = self.parse_construct_real_variadic()?;
            if a.len() % 4 != 0 {
                return Err(VariantParseError("PackedColorArray requires multiple of 4 components".into()));
            }
            let quads: Vec<_> = a
                .chunks(4)
                .map(|c| (RealT::F64(c[0]), RealT::F64(c[1]), RealT::F64(c[2]), RealT::F64(c[3])))
                .collect();
            return Ok(VariantVal::PackedColorArray(quads));
        }

        Err(VariantParseError(format!("Unexpected identifier '{}'", id)))
    }

    fn parse_construct_int_variadic(&mut self) -> Result<Vec<i32>, VariantParseError> {
        self.expect("(")?;
        let mut args = Vec::new();
        let mut first = true;
        loop {
            if !first {
                let t = self.advance()?;
                if matches!(t, Token::ParenClose) {
                    break;
                }
                if !matches!(t, Token::Comma) {
                    return Err(VariantParseError("Expected ',' or ')'".into()));
                }
            }
            let t = self.advance()?;
            if first && matches!(t, Token::ParenClose) {
                break;
            }
            let i = match &t {
                Token::Number { int, float } => int.unwrap_or(*float as i64) as i32,
                Token::Identifier(x) => stor_fix(x).map(|f| f as i32).unwrap_or(0),
                _ => return Err(VariantParseError("Expected number".into())),
            };
            args.push(i);
            first = false;
        }
        Ok(args)
    }

    fn parse_construct_real_variadic(&mut self) -> Result<Vec<f64>, VariantParseError> {
        self.expect("(")?;
        let mut args = Vec::new();
        let mut first = true;
        loop {
            if !first {
                let t = self.advance()?;
                if matches!(t, Token::ParenClose) {
                    break;
                }
                if !matches!(t, Token::Comma) {
                    return Err(VariantParseError("Expected ',' or ')'".into()));
                }
            }
            let t = self.advance()?;
            if first && matches!(t, Token::ParenClose) {
                break;
            }
            let f = match &t {
                Token::Number { float, .. } => *float,
                Token::Identifier(x) => stor_fix(x).ok_or_else(|| VariantParseError("Expected number".into()))?,
                _ => return Err(VariantParseError("Expected number".into())),
            };
            args.push(f);
            first = false;
        }
        Ok(args)
    }

    fn parse_object(&mut self) -> Result<VariantVal, VariantParseError> {
        self.expect("(")?;
        let t = self.advance()?;
        let type_name = match &t {
            Token::Identifier(s) => s.clone(),
            _ => return Err(VariantParseError("Expected identifier with type of object".into())),
        };
        self.expect(",")?;
        let mut props = IndexMap::new();
        loop {
            let key_tok = self.advance()?;
            if matches!(key_tok, Token::ParenClose) {
                break;
            }
            if !matches!(key_tok, Token::Str(..)) {
                return Err(VariantParseError("Expected property name as string".into()));
            }
            let key = match key_tok {
                Token::Str(k) => k,
                _ => unreachable!(),
            };
            self.expect(":")?;
            let val_tok = self.advance()?;
            let val = self.parse_value(val_tok)?;
            props.insert(key, Box::new(val));
            let next = self.get_token()?;
            if matches!(next, Token::ParenClose) {
                continue;
            }
            if !matches!(next, Token::Comma) {
                return Err(VariantParseError("Expected '}' or ','".into()));
            }
            self.advance()?;
        }
        Ok(VariantVal::Object(type_name, props))
    }

    fn parse_resource(&mut self, id: &str) -> Result<VariantVal, VariantParseError> {
        self.expect("(")?;
        let t = self.advance()?;
        match id {
            "Resource" => {
                let (path, uid) = match &t {
                    Token::Str(uid_or_path) => {
                        let uid_or_path = uid_or_path.clone();
                        let next = self.get_token()?;
                        if matches!(next, Token::Comma) {
                            self.advance()?;
                            let t2 = self.advance()?;
                            let path = match &t2 {
                                Token::Str(u) => u.clone(),
                                _ => return Err(VariantParseError("Expected string in Resource reference".into())),
                            };
                            (path, Some(uid_or_path))
                        } else {
                            (uid_or_path, None)
                        }
                    }
                    _ => return Err(VariantParseError("Expected string as argument for Resource()".into())),
                };
                self.expect(")")?;
                Ok(VariantVal::Resource(uid, path))
            }
            "SubResource" => {
                let id_str = match &t {
                    Token::Str(s) => s.clone(),
                    _ => return Err(VariantParseError("Expected identifier for SubResource".into())),
                };
                self.expect(")")?;
                Ok(VariantVal::SubResource(id_str))
            }
            "ExtResource" => {
                let (id_str, path, uid) = match &t {
                    Token::Str(path) => {
                        let path = path.clone();
                        let next = self.advance()?;
                        if matches!(next, Token::ParenClose) {
                            (String::new(), path, None)
                        } else if matches!(next, Token::Comma) {
                            let t2 = self.advance()?;
                            match &t2 {
                                Token::Str(uid_or_path) => {
                                    let uid_or_path = uid_or_path.clone();
                                    let (uid, path) = if uid_or_path.starts_with("uid://") {
                                        (Some(uid_or_path), String::new())
                                    } else {
                                        (None, uid_or_path)
                                    };
                                    (String::new(), path, uid)
                                }
                                _ => return Err(VariantParseError("Expected string".into())),
                            }
                        } else {
                            return Err(VariantParseError("Expected ',' or ')'".into()));
                        }
                    }
                    Token::Identifier(id_str) => {
                        let id_str = id_str.clone();
                        self.expect(",")?;
                        let t2 = self.advance()?;
                        let path = match &t2 {
                            Token::Str(p) => p.clone(),
                            _ => return Err(VariantParseError("Expected path string".into())),
                        };
                        let tok = self.get_token()?.clone();
                        let uid =  if matches!(tok, Token::Comma) {
                                self.advance()?;
                                let t3 = self.advance()?;
                                match &t3 {
                                    Token::Str(u) if u.starts_with("uid://") => Some(u.clone()),
                                    _ => None,
                                }
                            } else {
                                None
                            } ;
                        (id_str, path, uid)
                    }
                    _ => return Err(VariantParseError("Expected string or identifier for ExtResource".into())),
                };
                self.expect(")")?;
                Ok(VariantVal::ExtResource(id_str, uid, path))
            }
            _ => unreachable!(),
        }
    }

    fn parse_typed_dictionary(&mut self) -> Result<VariantVal, VariantParseError> {
        self.expect("[")?;
        let _key_type = self.advance()?;
        let key_type = match &_key_type {
            Token::Identifier(s) => self.parse_type_identifier(s)?,
            _ => return Err(VariantParseError("Expected identifier for key type".into())),
        };
        self.expect(",")?;
        let _val_type = self.advance()?;
        let val_type = match &_val_type {
            Token::Identifier(s) => self.parse_type_identifier(s)?,
            _ => return Err(VariantParseError("Expected identifier for value type".into())),
        };
        self.expect("]")?;
        self.expect("(")?;
        self.expect("{")?;
        let map = self.parse_dictionary_body()?;
        self.expect(")")?;
        Ok(VariantVal::Dictionary(Some((Box::new(key_type), Box::new(val_type))), map))
    }
    fn parse_type_identifier(&mut self, s: &str) -> Result<ElemType, VariantParseError> {
        let id = if s == "Resource" || s == "SubResource" || s == "ExtResource" {
            self.parse_identifier(s)
        } else {
            Ok(VariantVal::String(s.to_string()))
        };
        match id {
            Ok(VariantVal::String(s)) => Ok(ElemType::Identifier(s)),
            Ok(VariantVal::Resource(uid, path)) => Ok(ElemType::Resource(uid, path)),
            Ok(VariantVal::SubResource(s)) => Ok(ElemType::SubResource(s)),
            Ok(VariantVal::ExtResource(id, uid, path)) => Ok(ElemType::ExtResource(id, uid, path)),
            _ => Err(VariantParseError("Expected identifier for type".into())),
        }
    }

    fn parse_typed_array(&mut self) -> Result<VariantVal, VariantParseError> {
        self.expect("[")?;
        let _elem_type = self.advance()?; // skip type identifier
        let elem_type = match &_elem_type {
            Token::Identifier(s) => self.parse_type_identifier(s)?,
            _ => return Err(VariantParseError("Expected identifier for element type".into())),
        };
        self.expect("]")?;
        self.expect("(")?;
        self.expect("[")?;
        let arr = self.parse_array_body()?;
        self.expect("]")?;
        self.expect(")")?;
        Ok(VariantVal::Array(Some(Box::new(elem_type)), arr))
    }

    fn parse_packed_byte_array(&mut self) -> Result<VariantVal, VariantParseError> {
        self.expect("(")?;
        let t = self.advance()?;
        match &t {
            Token::Str(base64) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64.as_bytes())
                    .map_err(|_| VariantParseError("Invalid base64-encoded string".into()))?;
                self.expect(")")?;
                Ok(VariantVal::PackedByteArray(bytes))
            }
            Token::ParenClose => Ok(VariantVal::PackedByteArray(Vec::new())),
            Token::Number { .. } | Token::Identifier(_) => {
                let mut bytes = Vec::new();
                let mut tok = t;
                loop {
                    let b = match &tok {
                        Token::Number { int, float } => int.unwrap_or(*float as i64) as u8,
                        Token::Identifier(x) => stor_fix(x).map(|f| f as u8).unwrap_or(0),
                        _ => return Err(VariantParseError("Expected number in constructor".into())),
                    };
                    bytes.push(b);
                    let next = self.advance()?;
                    if matches!(next, Token::ParenClose) {
                        break;
                    }
                    if !matches!(next, Token::Comma) {
                        return Err(VariantParseError("Expected ',' or ')'".into()));
                    }
                    tok = self.advance()?;
                }
                Ok(VariantVal::PackedByteArray(bytes))
            }
            _ => Err(VariantParseError("Expected base64 string or list of numbers".into())),
        }
    }
}

// -----------------------------------------------------------------------------
// FromStr
// -----------------------------------------------------------------------------

impl FromStr for VariantVal {
    type Err = VariantParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parser = Parser::new(s.trim());
        let first = parser.advance()?;
        if matches!(first, Token::Eof) {
            return Err(VariantParseError("Expected value".into()));
        }
        let value = parser.parse_value(first)?;
        let next = parser.get_token()?;
        if !matches!(next, Token::Eof) {
            return Err(VariantParseError("Unexpected trailing input".into()));
        }
        Ok(value)
    }
}

// -----------------------------------------------------------------------------
// Display (variant text format, inverse of FromStr)
// -----------------------------------------------------------------------------

impl Display for ElemType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ElemType::Identifier(s) => write!(f, "{}", s),
            ElemType::Resource(uid, path) => match uid {
                Some(uid) => write!(f, "Resource(\"{}\", \"{}\")", uid, path),
                None => write!(f, "Resource(\"{}\")", path),
            },
            ElemType::SubResource(s) => write!(f, "SubResource(\"{}\")", s),
            ElemType::ExtResource(id, _, _) => write!(f, "ExtResource(\"{}\")", id),
        }
    }
}

impl VariantVal {
    pub fn to_string_compat(&self, compat: bool) -> Result<String, std::fmt::Error> {
        let mut output = String::new();
        self.value_to_string_write(&mut output, compat)?;
        Ok(output)
    }

    fn value_to_string_write(&self, f: &mut dyn std::fmt::Write, compat: bool) -> std::fmt::Result {
        use VariantVal::*;

        match self {
            Nil => write!(f, "null"),
            Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            Int(i) => write!(f, "{}", i),
            Float(x) => write!(f, "{}", format_float_rtos_fix(*x, compat)),
            String(s) => write!(f, "\"{}\"", escape_string_for_variant(s)),
            Vector2(x, y) => write!(f, "Vector2({}, {})", format_float_rtos_fix(x.to_f64(), compat), format_float_rtos_fix(y.to_f64(), compat)),
            Vector2i(x, y) => write!(f, "Vector2i({}, {})", x, y),
            Rect2(p, sz) => write!(f, "Rect2({}, {}, {}, {})", format_float_rtos_fix(p.0.to_f64(), compat), format_float_rtos_fix(p.1.to_f64(), compat), format_float_rtos_fix(sz.0.to_f64(), compat), format_float_rtos_fix(sz.1.to_f64(), compat)),
            Rect2i((px, py), (sx, sy)) => write!(f, "Rect2i({}, {}, {}, {})", px, py, sx, sy),
            Vector3(x, y, z) => write!(f, "Vector3({}, {}, {})", format_float_rtos_fix(x.to_f64(), compat), format_float_rtos_fix(y.to_f64(), compat), format_float_rtos_fix(z.to_f64(), compat)),
            Vector3i(x, y, z) => write!(f, "Vector3i({}, {}, {})", x, y, z),
            Transform2d(c0, c1, c2) => write!(f, "Transform2D({}, {}, {}, {}, {}, {})", format_float_rtos_fix(c0.0.to_f64(), compat), format_float_rtos_fix(c0.1.to_f64(), compat), format_float_rtos_fix(c1.0.to_f64(), compat), format_float_rtos_fix(c1.1.to_f64(), compat), format_float_rtos_fix(c2.0.to_f64(), compat), format_float_rtos_fix(c2.1.to_f64(), compat)),
            Vector4(x, y, z, w) => write!(f, "Vector4({}, {}, {}, {})", format_float_rtos_fix(x.to_f64(), compat), format_float_rtos_fix(y.to_f64(), compat), format_float_rtos_fix(z.to_f64(), compat), format_float_rtos_fix(w.to_f64(), compat)),
            Vector4i(x, y, z, w) => write!(f, "Vector4i({}, {}, {}, {})", x, y, z, w),
            Plane((nx, ny, nz), d) => write!(f, "Plane({}, {}, {}, {})", format_float_rtos_fix(nx.to_f64(), compat), format_float_rtos_fix(ny.to_f64(), compat), format_float_rtos_fix(nz.to_f64(), compat), format_float_rtos_fix(d.to_f64(), compat)),
            Quaternion(x, y, z, w) => write!(f, "Quaternion({}, {}, {}, {})", format_float_rtos_fix(x.to_f64(), compat), format_float_rtos_fix(y.to_f64(), compat), format_float_rtos_fix(z.to_f64(), compat), format_float_rtos_fix(w.to_f64(), compat)),
            Aabb(p, s) => write!(f, "AABB({}, {}, {}, {}, {}, {})", format_float_rtos_fix(p.0.to_f64(), compat), format_float_rtos_fix(p.1.to_f64(), compat), format_float_rtos_fix(p.2.to_f64(), compat), format_float_rtos_fix(s.0.to_f64(), compat), format_float_rtos_fix(s.1.to_f64(), compat), format_float_rtos_fix(s.2.to_f64(), compat)),
            Basis(r0, r1, r2) => write!(f, "Basis({}, {}, {}, {}, {}, {}, {}, {}, {})", format_float_rtos_fix(r0.0.to_f64(), compat), format_float_rtos_fix(r0.1.to_f64(), compat), format_float_rtos_fix(r0.2.to_f64(), compat), format_float_rtos_fix(r1.0.to_f64(), compat), format_float_rtos_fix(r1.1.to_f64(), compat), format_float_rtos_fix(r1.2.to_f64(), compat), format_float_rtos_fix(r2.0.to_f64(), compat), format_float_rtos_fix(r2.1.to_f64(), compat), format_float_rtos_fix(r2.2.to_f64(), compat)),
            Transform3d((r0, r1, r2), o) => write!(f, "Transform3D({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})", format_float_rtos_fix(r0.0.to_f64(), compat), format_float_rtos_fix(r0.1.to_f64(), compat), format_float_rtos_fix(r0.2.to_f64(), compat), format_float_rtos_fix(r1.0.to_f64(), compat), format_float_rtos_fix(r1.1.to_f64(), compat), format_float_rtos_fix(r1.2.to_f64(), compat), format_float_rtos_fix(r2.0.to_f64(), compat), format_float_rtos_fix(r2.1.to_f64(), compat), format_float_rtos_fix(r2.2.to_f64(), compat), format_float_rtos_fix(o.0.to_f64(), compat), format_float_rtos_fix(o.1.to_f64(), compat), format_float_rtos_fix(o.2.to_f64(), compat)),
            Projection(c0, c1, c2, c3) => write!(f, "Projection({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})", format_float_rtos_fix(c0.0.to_f64(), compat), format_float_rtos_fix(c0.1.to_f64(), compat), format_float_rtos_fix(c0.2.to_f64(), compat), format_float_rtos_fix(c0.3.to_f64(), compat), format_float_rtos_fix(c1.0.to_f64(), compat), format_float_rtos_fix(c1.1.to_f64(), compat), format_float_rtos_fix(c1.2.to_f64(), compat), format_float_rtos_fix(c1.3.to_f64(), compat), format_float_rtos_fix(c2.0.to_f64(), compat), format_float_rtos_fix(c2.1.to_f64(), compat), format_float_rtos_fix(c2.2.to_f64(), compat), format_float_rtos_fix(c2.3.to_f64(), compat), format_float_rtos_fix(c3.0.to_f64(), compat), format_float_rtos_fix(c3.1.to_f64(), compat), format_float_rtos_fix(c3.2.to_f64(), compat), format_float_rtos_fix(c3.3.to_f64(), compat)),
            Color(r, g, b, a) => write!(f, "Color({}, {}, {}, {})", format_float_rtos_fix(*r as f64, compat), format_float_rtos_fix(*g as f64, compat), format_float_rtos_fix(*b as f64, compat), format_float_rtos_fix(*a as f64, compat)),
            StringName(s) => write!(f, "&\"{}\"", escape_string_for_variant(s)),
            NodePath(s) => write!(f, "NodePath(\"{}\")", escape_string_for_variant(s)),
            Rid(id) => if id.is_empty() { write!(f, "RID()") } else { write!(f, "RID({})", id) },
            Object(ty, props) => {
                write!(f, "Object({}, ", ty)?;
                let mut first = true;
                for (k, v) in props {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "\"{}\": {}", escape_string_for_variant(k), v)?;
                }
                write!(f, ")")
            }
            Callable => write!(f, "Callable()"),
            Signal => write!(f, "Signal()"),
            Dictionary(typed, map) => {
                if let Some((key_type, value_type)) = typed {
                    write!(f, "Dictionary[{}, {}](", key_type.to_string(), value_type.to_string())?;
                }
                write!(f, "{{")?;
                let size = map.len();
                if size > 0 {
                    write!(f, "\n")?;
                }
                for (i, (key, value)) in map.iter().enumerate() {
                    write!(f, "{}: {}", key.to_string_compat(compat)?, value.to_string_compat(compat)?)?;
                    if i < size - 1 {
                        write!(f, ",")?;
                    }
                    write!(f, "\n")?;
                }
                write!(f, "}}")?;
                if typed.is_some() {
                    write!(f, ")")?;
                }
                Ok(())
            }
            Array(elem_type, arr) => {
                if let Some(elem_type) = elem_type {
                    write!(f, "Array[{}](", elem_type.to_string())?;
                }
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")?;
                if elem_type.is_some() {
                    write!(f, ")")?;
                }
                Ok(())
            }
            PackedByteArray(bytes) => {
                write!(f, "PackedByteArray(")?;
                if bytes.is_empty() {
                    write!(f, ")")?;
                } else if compat {
                    // write the bytes as a list of numbers
                    for (i, b) in bytes.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", b)?;
                    }
                } else {
                    write!(f, "\"{}\")", base64::engine::general_purpose::STANDARD.encode(bytes))?;
                }
                Ok(())
            }
            PackedInt32Array(arr) => {
                write!(f, "PackedInt32Array(")?;
                for (i, x) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", x)?;
                }
                write!(f, ")")
            }
            PackedInt64Array(arr) => {
                write!(f, "PackedInt64Array(")?;
                for (i, x) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", x)?;
                }
                write!(f, ")")
            }
            PackedFloat32Array(arr) => {
                write!(f, "PackedFloat32Array(")?;
                for (i, x) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", format_float_rtos_fix(*x as f64, compat))?;
                }
                write!(f, ")")
            }
            PackedFloat64Array(arr) => {
                write!(f, "PackedFloat64Array(")?;
                for (i, x) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", format_float_rtos_fix(*x, compat))?;
                }
                write!(f, ")")
            }
            PackedStringArray(arr) => {
                write!(f, "PackedStringArray(")?;
                for (i, s) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\"", escape_string_for_variant(s))?;
                }
                write!(f, ")")
            }
            PackedVector2Array(arr) => {
                write!(f, "PackedVector2Array(")?;
                for (i, (x, y)) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}, {}", format_float_rtos_fix(x.to_f64(), compat), format_float_rtos_fix(y.to_f64(), compat))?;
                }
                write!(f, ")")
            }
            PackedVector3Array(arr) => {
                write!(f, "PackedVector3Array(")?;
                for (i, (x, y, z)) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}, {}, {}", format_float_rtos_fix(x.to_f64(), compat), format_float_rtos_fix(y.to_f64(), compat), format_float_rtos_fix(z.to_f64(), compat))?;
                }
                write!(f, ")")
            }
            PackedVector4Array(arr) => {
                write!(f, "PackedVector4Array(")?;
                for (i, (x, y, z, w)) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}, {}, {}, {}", format_float_rtos_fix(x.to_f64(), compat), format_float_rtos_fix(y.to_f64(), compat), format_float_rtos_fix(z.to_f64(), compat), format_float_rtos_fix(w.to_f64(), compat))?;
                }
                write!(f, ")")
            }
            PackedColorArray(arr) => {
                write!(f, "PackedColorArray(")?;
                for (i, (r, g, b, a)) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}, {}, {}, {}", format_float_rtos_fix(r.to_f64(), compat), format_float_rtos_fix(g.to_f64(), compat), format_float_rtos_fix(b.to_f64(), compat), format_float_rtos_fix(a.to_f64(), compat))?;
                }
                write!(f, ")")
            }
            Resource(uid_opt, path) => {
                if let Some(uid) = uid_opt {
                    write!(f, "Resource(\"{}\", \"{}\")", escape_string_for_variant(uid), escape_string_for_variant(path))
                } else {
                    write!(f, "Resource(\"{}\")", escape_string_for_variant(path))
                }
            }
            SubResource(id) => write!(f, "SubResource(\"{}\")", escape_string_for_variant(id)),
            ExtResource(id, _, _) => {
                write!(f, "ExtResource(\"{}\")", escape_string_for_variant(id))
            }
        }
        }
}

impl Display for VariantVal {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value_to_string_write(f, false)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
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
            ("null", VariantVal::Nil, true),
            ("nil", VariantVal::Nil, false),
            ("true", VariantVal::Bool(true), true),
            ("false", VariantVal::Bool(false), true),
            ("123", VariantVal::Int(123), true),
            ("123.456", VariantVal::Float(123.456), true),
            // scientific notation
            ("1.23456e+10", VariantVal::Float(1.23456e+10), true),
            ("1.23456e-10", VariantVal::Float(1.23456e-10), true),
            ("inf", VariantVal::Float(f64::INFINITY), true  ),
            ("-inf", VariantVal::Float(f64::NEG_INFINITY), true),
            ("nan", VariantVal::Float(f64::NAN), true),
            ("\"foo\"", VariantVal::String("foo".into()), true),
            ("&\"foo\"", VariantVal::StringName("foo".into()), true),
            ("#ff0000", VariantVal::Color(1.0, 0.0, 0.0, 1.0), false),
            ("#ff000080", VariantVal::Color(1.0, 0.0, 0.0, 128.0 / 255.0), false),
            ("#f00", VariantVal::Color(1.0, 0.0, 0.0, 1.0), false), // 3-digit (Godot Color::html)
            ("#f008", VariantVal::Color(1.0, 0.0, 0.0, 8.0 / 15.0), false), // 4-digit
            ("Vector2(1.0, 2.0)", VariantVal::Vector2(RealT::F64(1.0), RealT::F64(2.0)), true),
            ("Vector2i(1, 2)", VariantVal::Vector2i(1, 2), true),
            ("Rect2(0, 0, 10.0, 10.0)", VariantVal::Rect2(
                (RealT::F64(0.0), RealT::F64(0.0)),
                (RealT::F64(10.0), RealT::F64(10.0)),
            ), true     ),
            ("Rect2i(0, 0, 10, 10)", VariantVal::Rect2i((0, 0), (10, 10)), true ),
            ("Vector3(1.0, 2.0, 3.0)", VariantVal::Vector3(RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0)), true),
            ("Vector3i(1, 2, 3)", VariantVal::Vector3i(1, 2, 3), true ),
            ("Vector4(1.0, 2.0, 3.0, 4.0)", VariantVal::Vector4(RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0), RealT::F64(4.0)), true ),
            ("Vector4i(1, 2, 3, 4)", VariantVal::Vector4i(1, 2, 3, 4), true ),
            (
                "Transform2D(1.0, 0, 0, 1.0, 0, 0)",
                VariantVal::Transform2d(
                    (RealT::F64(1.0), RealT::F64(0.0)),
                    (RealT::F64(0.0), RealT::F64(1.0)),
                    (RealT::F64(0.0), RealT::F64(0.0)),
                ),
                true,
            ),
            ("Plane(1.0, 0, 0, 0)", VariantVal::Plane(
                (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0)),
                RealT::F64(0.0),
            ), true),
            ("Quaternion(1.0, 0, 0, 0)", VariantVal::Quaternion(
                RealT::F64(1.0),
                RealT::F64(0.0),
                RealT::F64(0.0),
                RealT::F64(0.0),
            ), true),
            (
                "AABB(0, 0, 0, 1.0, 1.0, 1.0)",
                VariantVal::Aabb(
                    (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(0.0)),
                    (RealT::F64(1.0), RealT::F64(1.0), RealT::F64(1.0)),
                ), true
            ),
            (
                "Basis(1.0, 0, 0, 0, 1.0, 0, 0, 0, 1.0)",
                VariantVal::Basis(
                    (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0)),
                    (RealT::F64(0.0), RealT::F64(1.0), RealT::F64(0.0)),
                    (RealT::F64(0.0), RealT::F64(0.0), RealT::F64(1.0)),
                ),
                true,
            ),
            (
                "Transform3D(1.0, 0, 0, 0, 1.0, 0, 0, 0, 1.0, 0, 0, 0)",
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
                "Color(1.0, 0, 0, 1.0)",
                VariantVal::Color(1.0, 0.0, 0.0, 1.0),
                true,
            ),
            ("NodePath(\"foo/bar/baz\")", VariantVal::NodePath("foo/bar/baz".into()), true),
            ("RID()", VariantVal::Rid("".into()), true),
            ("RID(42)", VariantVal::Rid("42".into()), true),
            ("Callable()", VariantVal::Callable, true),
            ("Signal()", VariantVal::Signal, true),
            (
                "Object(Node, \"bar\": 123)",
                VariantVal::Object("Node".into(), object_props),
                true,
            ),
            ("{\n\"foo\": \"bar\",\n\"baz\": 123\n}", VariantVal::Dictionary(None, map_dict), true),
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
                VariantVal::Dictionary(Some((Box::new(ElemType::Identifier("String".into())), Box::new(ElemType::Identifier("int".into())))), map_typed_dict),
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
                "PackedFloat32Array(1.0, 2.0, 3.0)",
                VariantVal::PackedFloat32Array(vec![1.0, 2.0, 3.0]),
                true,
            ),
            (
                "PackedFloat64Array(1.0, 2.0, 3.0)",
                VariantVal::PackedFloat64Array(vec![1.0, 2.0, 3.0]),
                true,
            ),
            (
                "PackedStringArray(\"a\", \"b\", \"c\")",
                VariantVal::PackedStringArray(vec!["a".into(), "b".into(), "c".into()]),
                true,
            ),
            (
                "PackedVector2Array(1.0, 2.0, 3.0, 4.0)",
                VariantVal::PackedVector2Array(vec![
                    (RealT::F64(1.0), RealT::F64(2.0)),
                    (RealT::F64(3.0), RealT::F64(4.0)),
                ]),
                true,
            ),
            (
                "PackedVector3Array(1.0, 2.0, 3.0, 4.0, 5.0, 6.0)",
                VariantVal::PackedVector3Array(vec![
                    (RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0)),
                    (RealT::F64(4.0), RealT::F64(5.0), RealT::F64(6.0)),
                ]),
                true,
            ),
            (
                "PackedVector4Array(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0)",
                VariantVal::PackedVector4Array(vec![
                    (RealT::F64(1.0), RealT::F64(2.0), RealT::F64(3.0), RealT::F64(4.0)),
                    (RealT::F64(5.0), RealT::F64(6.0), RealT::F64(7.0), RealT::F64(8.0)),
                ]),
                true,
            ),
            (
                "PackedColorArray(1.0, 0, 0, 1.0, 0, 1.0, 0, 1.0)",
                VariantVal::PackedColorArray(vec![
                    (RealT::F64(1.0), RealT::F64(0.0), RealT::F64(0.0), RealT::F64(1.0)),
                    (RealT::F64(0.0), RealT::F64(1.0), RealT::F64(0.0), RealT::F64(1.0)),
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
            ("SubResource(\"foo\")", VariantVal::SubResource("foo".into()), true),
            // (
            //     "ExtResource(\"res://bar.tres\")",
            //     VariantVal::ExtResource("".into(), None, "res://bar.tres".into()),
            // ),
            // (
            //     "ExtResource(id, \"res://bar.tres\")",
            //     VariantVal::ExtResource("id".into(), None, "res://bar.tres".into()),
            // ),
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
                assert_eq!(expected.to_string_compat(false).unwrap(), input, "input: {:?}", input);
            }
        }
    }
}
use std::{collections::HashMap, fmt::{Display, Formatter}};

use autosurgeon::{Hydrate, Reconcile};

use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize, Hydrate, Reconcile, PartialEq, Eq, Hash)]
pub struct OrderedProperty {
    pub value: VariantVal,
    pub order: i64,
}

impl OrderedProperty {
    pub fn new(value: VariantVal, order: i64) -> Self {
        Self { value, order }
    }
	pub fn get_value(&self) -> VariantVal {
		self.value.clone()
	}
}

impl Display for OrderedProperty{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

#[derive(Clone, Debug, Hydrate, Reconcile, PartialEq, Serialize, Deserialize)]
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

impl From<f32> for RealT {
    fn from(f: f32) -> Self {
        RealT::F32(f)
    }
}

#[derive(Clone, Debug, Hydrate, Reconcile, Serialize, Deserialize)]
pub enum ElemType {
    Identifier(String),
    Resource(Option<String>, String),
    SubResource(String),
    ExtResource(String),
}



impl PartialEq for ElemType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ElemType::Identifier(a), ElemType::Identifier(b)) => a == b,
            (ElemType::Resource(a, b), ElemType::Resource(c, d)) => a == c && b == d,
            (ElemType::SubResource(a), ElemType::SubResource(b)) => a == b,
            (ElemType::ExtResource(a), ElemType::ExtResource(b)) => a == b,
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
            ElemType::ExtResource(a) => {
                a.hash(state);
            }
        }
    }
}


#[derive(Clone, Debug, Hydrate, Reconcile, Serialize, Deserialize)]
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
    Object(String, HashMap<String, OrderedProperty>),
    Callable,
    Signal,
    Dictionary(
        Option<(ElemType, ElemType)>,
        Vec<(OrderedProperty, OrderedProperty)>,
    ),
    Array(Option<ElemType>, Vec<OrderedProperty>),
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
    ExtResource(String),
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
            (ExtResource(ia), ExtResource(ib)) => ia == ib,
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
            VariantVal::ExtResource(id) => id.hash(state),
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

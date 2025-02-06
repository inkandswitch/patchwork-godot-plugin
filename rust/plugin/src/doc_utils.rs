use automerge::{transaction::Transaction, Automerge, ObjId, Prop, ReadDoc, Value};

pub trait SimpleDocReader {
    fn get_bytes<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<Vec<u8>>;

    fn get_int<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<i64>;

    fn get_string<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<String>;

    fn get_obj_id<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<ObjId>;
}

impl SimpleDocReader for Automerge {
    fn get_bytes<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<Vec<u8>> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::Bytes(bytes) => Some(bytes),
                _ => None,
            },
            _ => None,
        }
    }

    fn get_int<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<i64> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::Int(num) => Some(num),
                _ => None,
            },
            _ => None,
        }
    }

    fn get_string<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<String> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::Str(smol_str) => Some(smol_str.to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    fn get_obj_id<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<ObjId> {
        match self.get(obj, prop) {
            Ok(Some((Value::Object(_), obj_id))) => Some(obj_id),
            _ => None,
        }
    }
}

impl SimpleDocReader for Transaction<'_> {
    fn get_bytes<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<Vec<u8>> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::Bytes(bytes) => Some(bytes),
                _ => None,
            },
            _ => None,
        }
    }

    fn get_int<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<i64> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::Int(num) => Some(num),
                _ => None,
            },
            _ => None,
        }
    }

    fn get_string<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<String> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::Str(smol_str) => Some(smol_str.to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    fn get_obj_id<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<ObjId> {
        match self.get(obj, prop) {
            Ok(Some((Value::Object(_), obj_id))) => Some(obj_id),
            _ => None,
        }
    }
}
use automerge::{Automerge, ObjId, Prop, ReadDoc, Value};

pub trait SimpleDocReader {
    fn get_string<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<String>;

    fn get_bytes<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<Vec<u8>>;
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

    fn get_string<O: AsRef<ObjId>, P: Into<Prop>>(&self, obj: O, prop: P) -> Option<String> {
        match self.get(obj, prop) {
            Ok(Some((Value::Scalar(cow), _))) => match cow.into_owned() {
                automerge::ScalarValue::Str(smol_str) => Some(smol_str.to_string()),
                _ => None,
            },
            _ => None,
        }
    }
}

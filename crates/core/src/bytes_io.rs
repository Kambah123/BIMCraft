//! In-memory file bytes for a command that would otherwise read from a path.
//!
//! The desktop commands (`open`, `import_cad`) take a filesystem path. The browser has no
//! path that `std::fs` can read, so a front end that already has the file's bytes (the File
//! API, a drop, a test) parks them here under that path's name; the command takes them
//! instead of opening the file. Native callers that never [`provide`] are unchanged.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

thread_local! {
    static INBOX: RefCell<HashMap<PathBuf, Vec<u8>>> = RefCell::new(HashMap::new());
}

/// Park `bytes` so the next `open` or `import_cad` that names `path` reads them instead of the disk.
pub fn provide(path: impl AsRef<Path>, bytes: Vec<u8>) {
    let path = path.as_ref().to_path_buf();
    INBOX.with(|b| b.borrow_mut().insert(path, bytes));
}

/// Take the bytes previously [`provide`]d for `path`, if any.
pub fn take(path: impl AsRef<Path>) -> Option<Vec<u8>> {
    INBOX.with(|b| b.borrow_mut().remove(path.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provide_then_take_is_once() {
        provide("house.bimcraft", b"{}".to_vec());
        assert_eq!(take("house.bimcraft"), Some(b"{}".to_vec()));
        assert_eq!(take("house.bimcraft"), None);
    }

    #[test]
    fn open_and_encode_roundtrip_without_a_path() {
        use crate::engine::Session;
        use serde_json::json;

        let mut s = Session::new();
        s.script(crate::sample::SAMPLE).unwrap();
        let (bytes, reply) = s.encode("bimcraft", &json!({})).unwrap();
        assert_eq!(reply["format"], "bimcraft");
        assert!(!bytes.is_empty());

        let mut opened = Session::new();
        opened.provide_bytes("house.bimcraft", bytes);
        opened.execute("open", &json!({"path": "house.bimcraft"})).unwrap();
        assert_eq!(opened.model.name, s.model.name);

        let (ifc, reply) = s.encode("ifc", &json!({})).unwrap();
        assert_eq!(reply["format"], "IFC4");
        assert!(std::str::from_utf8(&ifc).unwrap().contains("ISO-10303-21"));

        let mut from_ifc = Session::new();
        from_ifc.provide_bytes("house.ifc", ifc);
        let reply = from_ifc.execute("open", &json!({"path": "house.ifc"})).unwrap();
        assert_eq!(reply["format"], "IFC4");

        let (png, reply) = s.encode("png", &json!({"width": 64, "height": 64})).unwrap();
        assert!(reply.get("pngBase64").is_some());
        assert!(png.starts_with(b"\x89PNG") || png.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}

//! A CO-EDITED DOCUMENT (ARCHITECTURE §7 "Live co-editing"): one CRDT — Automerge (vendor/automerge-0.12.0) — the same
//! in the page and the node. Each editor's device writes its CHANGES as rows of its OWN feed in the document's table
//! (`storage`: every writer its own feed); every reader LOADS every row, in any order, duplicates harmless, and all
//! reach the same document. Its MAKER writes the first change (the text object); everyone else starts from it, so no
//! two editors ever make two texts. Positions count UTF-16 units — what a page's text field counts.

use automerge::marks::{ExpandMark, Mark};
use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, ObjId, ObjType, ReadDoc, ScalarValue, Value, ROOT};

/// The document's text object's key at its root.
const TEXT: &str = "text";
/// Its CELLS: a map of keys to values, the last write winning per key — a sheet's cells (`c:A1`) and formats, a deck's
/// slides' fields (`s:<id>:title`). Made in the maker's first change with the text: never two maps.
const CELLS: &str = "cells";

pub struct Doc {
    am: AutoCommit,
}

/// A mark over a span: `[start, end)`, its name, its value (none: a flag, e.g. bold).
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub name: String,
    pub value: Option<String>,
}

impl Doc {
    /// A NEW document by its MAKER (`actor`: the editor's own id): its text object made — the first change, returned
    /// to be written as the maker's first row.
    pub fn create(actor: &[u8]) -> Result<(Doc, Vec<u8>), String> {
        let mut am = AutoCommit::new().with_actor(ActorId::from(actor));
        am.put_object(ROOT, TEXT, ObjType::Text)
            .map_err(|e| e.to_string())?;
        am.put_object(ROOT, CELLS, ObjType::Map)
            .map_err(|e| e.to_string())?;
        let first = am.save_incremental();
        Ok((Doc { am }, first))
    }

    /// The document from its ROWS (every editor's changes, and a snapshot if one is kept), in any order — as
    /// `actor` edits it.
    pub fn load(actor: &[u8], rows: &[&[u8]]) -> Result<Doc, String> {
        let mut am = AutoCommit::new().with_actor(ActorId::from(actor));
        for r in rows {
            am.load_incremental(r).map_err(|e| e.to_string())?;
        }
        // (What this device writes next starts after what it read: nothing of it is sent again.)
        let _ = am.save_incremental();
        Ok(Doc { am })
    }

    /// ROWS read later (another editor's): taken in. Returns whether the document changed.
    pub fn apply(&mut self, rows: &[&[u8]]) -> Result<bool, String> {
        let before = self.am.get_heads();
        for r in rows {
            self.am.load_incremental(r).map_err(|e| e.to_string())?;
        }
        let _ = self.am.save_incremental();
        Ok(self.am.get_heads() != before)
    }

    fn text_obj(&self) -> Result<ObjId, String> {
        match self.am.get(ROOT, TEXT).map_err(|e| e.to_string())? {
            Some((Value::Object(ObjType::Text), id)) => Ok(id),
            _ => Err("no text in this document (its maker's first change not read yet)".into()),
        }
    }

    /// Its text now.
    pub fn text(&self) -> Result<String, String> {
        self.am.text(self.text_obj()?).map_err(|e| e.to_string())
    }

    /// An EDIT: `delete` units at `pos` removed, `insert` put there. Returns the change — this device's next row.
    pub fn splice(&mut self, pos: usize, delete: usize, insert: &str) -> Result<Vec<u8>, String> {
        let t = self.text_obj()?;
        self.am
            .splice_text(&t, pos, delete as isize, insert)
            .map_err(|e| e.to_string())?;
        Ok(self.am.save_incremental())
    }

    /// A MARK over `[start, end)` (bold, a link …; `value` none: a flag) — or taken off (`on` false). Returns the change.
    pub fn mark(
        &mut self,
        start: usize,
        end: usize,
        name: &str,
        value: Option<&str>,
        on: bool,
    ) -> Result<Vec<u8>, String> {
        let t = self.text_obj()?;
        if on {
            let v = value
                .map(|v| ScalarValue::Str(v.into()))
                .unwrap_or(ScalarValue::Boolean(true));
            self.am
                .mark(
                    &t,
                    Mark::new(name.to_string(), v, start, end),
                    ExpandMark::After,
                )
                .map_err(|e| e.to_string())?;
        } else {
            self.am
                .unmark(&t, name, start, end, ExpandMark::After)
                .map_err(|e| e.to_string())?;
        }
        Ok(self.am.save_incremental())
    }

    /// Its marks now.
    pub fn marks(&self) -> Result<Vec<Span>, String> {
        let t = self.text_obj()?;
        Ok(self
            .am
            .marks(t)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|m| Span {
                start: m.start,
                end: m.end,
                name: m.name().to_string(),
                value: match m.value() {
                    ScalarValue::Str(s) => Some(s.to_string()),
                    _ => None,
                },
            })
            .collect())
    }

    fn cells_obj(&self) -> Result<ObjId, String> {
        match self.am.get(ROOT, CELLS).map_err(|e| e.to_string())? {
            Some((Value::Object(ObjType::Map), id)) => Ok(id),
            _ => Err("no cells in this document (made before cells, or its maker's first change not read yet)".into()),
        }
    }

    /// A CELL set (or, `None`, cleared). Returns the change.
    pub fn set(&mut self, key: &str, value: Option<&str>) -> Result<Vec<u8>, String> {
        let c = self.cells_obj()?;
        match value {
            Some(v) => self.am.put(&c, key, v).map_err(|e| e.to_string())?,
            None => self.am.delete(&c, key).map_err(|e| e.to_string())?,
        }
        Ok(self.am.save_incremental())
    }

    /// Its cells now (key order), each the winning value of its key.
    pub fn cells(&self) -> Result<Vec<(String, String)>, String> {
        let c = match self.cells_obj() {
            Ok(c) => c,
            Err(_) => return Ok(Vec::new()),
        };
        let mut out = Vec::new();
        for k in self.am.keys(&c) {
            if let Some((Value::Scalar(v), _)) =
                self.am.get(&c, k.as_str()).map_err(|e| e.to_string())?
            {
                if let ScalarValue::Str(s) = v.as_ref() {
                    out.push((k.to_string(), s.to_string()));
                }
            }
        }
        Ok(out)
    }

    /// A SNAPSHOT: the whole document, compacted (kept as a FILE; its rows then need not be read).
    pub fn save(&mut self) -> Vec<u8> {
        self.am.save()
    }

    /// What it has seen (its heads, hex): two documents with the same heads are the same.
    pub fn heads(&mut self) -> Vec<String> {
        self.am.get_heads().iter().map(|h| h.to_string()).collect()
    }
}

#[cfg(test)]
mod tests;

/// FOR A PAGE (`doc-glue`, its own package — never in the core: only what co-edits loads it).
#[cfg(feature = "page")]
mod page {
    use wasm_bindgen::prelude::*;

    fn err(e: String) -> JsValue {
        JsValue::from_str(&e)
    }
    fn rows_of(rows: &js_sys::Array) -> Vec<Vec<u8>> {
        rows.iter()
            .map(|r| js_sys::Uint8Array::new(&r).to_vec())
            .collect()
    }

    /// A co-edited document (`super::Doc`), as a page holds it.
    #[wasm_bindgen]
    pub struct Doc(super::Doc);

    #[wasm_bindgen]
    impl Doc {
        /// A new document by its maker: `[doc, first change]`.
        pub fn create(actor: &[u8]) -> Result<js_sys::Array, JsValue> {
            let (d, first) = super::Doc::create(actor).map_err(err)?;
            Ok([
                JsValue::from(Doc(d)),
                js_sys::Uint8Array::from(&first[..]).into(),
            ]
            .into_iter()
            .collect())
        }
        /// The document from its rows (any order).
        pub fn load(actor: &[u8], rows: js_sys::Array) -> Result<Doc, JsValue> {
            let rows = rows_of(&rows);
            super::Doc::load(actor, &rows.iter().map(Vec::as_slice).collect::<Vec<_>>())
                .map(Doc)
                .map_err(err)
        }
        /// Rows read later: whether it changed.
        pub fn apply(&mut self, rows: js_sys::Array) -> Result<bool, JsValue> {
            let rows = rows_of(&rows);
            self.0
                .apply(&rows.iter().map(Vec::as_slice).collect::<Vec<_>>())
                .map_err(err)
        }
        pub fn text(&self) -> Result<String, JsValue> {
            self.0.text().map_err(err)
        }
        /// An edit: its change (this device's next row).
        pub fn splice(
            &mut self,
            pos: usize,
            delete: usize,
            insert: &str,
        ) -> Result<Vec<u8>, JsValue> {
            self.0.splice(pos, delete, insert).map_err(err)
        }
        /// A mark on or off: its change.
        pub fn mark(
            &mut self,
            start: usize,
            end: usize,
            name: &str,
            value: Option<String>,
            on: bool,
        ) -> Result<Vec<u8>, JsValue> {
            self.0
                .mark(start, end, name, value.as_deref(), on)
                .map_err(err)
        }
        /// Its marks: `[{ start, end, name, value }]` (JSON).
        pub fn marks(&self) -> Result<String, JsValue> {
            let ms = self.0.marks().map_err(err)?;
            Ok(format!(
                "[{}]",
                ms.iter()
                    .map(|m| format!(
                        r#"{{"start":{},"end":{},"name":{},"value":{}}}"#,
                        m.start,
                        m.end,
                        js_sys::JSON::stringify(&JsValue::from_str(&m.name))
                            .map(String::from)
                            .unwrap_or_default(),
                        m.value
                            .as_ref()
                            .map(|v| js_sys::JSON::stringify(&JsValue::from_str(v))
                                .map(String::from)
                                .unwrap_or_default())
                            .unwrap_or_else(|| "null".into())
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
        }
        /// A snapshot (kept as a file).
        pub fn save(&mut self) -> Vec<u8> {
            self.0.save()
        }
        /// A cell set (`value` none: cleared): its change.
        pub fn set(&mut self, key: &str, value: Option<String>) -> Result<Vec<u8>, JsValue> {
            self.0.set(key, value.as_deref()).map_err(err)
        }
        /// Its cells: `{ key: value }` (JSON).
        pub fn cells(&self) -> Result<String, JsValue> {
            let cs = self.0.cells().map_err(err)?;
            let o = js_sys::Object::new();
            for (k, v) in cs {
                js_sys::Reflect::set(&o, &JsValue::from_str(&k), &JsValue::from_str(&v))?;
            }
            js_sys::JSON::stringify(&o)
                .map(String::from)
                .map_err(|_| JsValue::from_str("cells not written"))
        }
        pub fn heads(&mut self) -> js_sys::Array {
            self.0.heads().into_iter().map(JsValue::from).collect()
        }
    }
}

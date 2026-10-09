//! Schema declarations. A domain's records are typed by its schema, and schemas
//! only ever grow: fields are appended, never changed or removed. That is what
//! makes "no migrations" true — old records stay valid under every later schema,
//! and old readers ignore fields they do not know.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Int,
    Float,
    Bool,
    /// Milliseconds since the epoch.
    Time,
    /// Hex in JSON.
    Bytes,
    /// A `craftec://` address of another record.
    Ref,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Field {
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub required: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Schema {
    /// Dictionary type name, e.g. `Task`, `Person`.
    #[serde(rename = "type")]
    pub type_name: String,
    pub fields: Vec<Field>,
    /// The field naming this record's PARENT, if records here are keyed under
    /// one.
    ///
    /// Records in such a domain are keyed `<parent>‖<rkey>` instead of
    /// `<rkey>`, so the children of one parent are a contiguous band and
    /// reading them is a bounded prefix scan rather than a scan of the whole
    /// domain filtered by a field (craftworks-sdk#122).
    ///
    /// This is the pattern the architecture already specifies twice — files as
    /// `f/<dir-id>/<name>`, edges as `e/<rel>/<from>/<to>` — and records are
    /// the primitive that did not get it.
    ///
    /// `Option`, defaulted and skipped when absent, because schemas are STORED
    /// as JSON: a domain defined before this existed must go on decoding, and
    /// its records must go on being keyed exactly as they were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

pub const MAX_FIELDS: usize = 256;
pub const MAX_NAME: usize = 64;

impl Schema {
    pub fn check(&self) -> Result<(), String> {
        if self.type_name.is_empty() || self.type_name.len() > MAX_NAME {
            return Err("schema type name must be 1–64 bytes".into());
        }
        if self.fields.len() > MAX_FIELDS {
            return Err(format!("at most {MAX_FIELDS} fields"));
        }
        if let Some(p) = &self.parent {
            let f = self.fields.iter().find(|f| &f.name == p).ok_or_else(|| {
                format!("parent field `{p}` is not one of this schema's fields")
            })?;
            if f.kind != Kind::Text {
                return Err(format!(
                    "parent field `{p}` must be text: it holds the parent's id"
                ));
            }
            if !f.required {
                return Err(format!(
                    "parent field `{p}` must be required: a record keyed under a \
                     parent cannot be written without one"
                ));
            }
        }
        for (i, f) in self.fields.iter().enumerate() {
            if f.name.is_empty() || f.name.len() > MAX_NAME {
                return Err(format!("field {i}: name must be 1–64 bytes"));
            }
            if self.fields[..i].iter().any(|g| g.name == f.name) {
                return Err(format!("duplicate field `{}`", f.name));
            }
        }
        Ok(())
    }

    /// May `next` replace `self`? Only by appending optional fields.
    pub fn allows(&self, next: &Schema) -> Result<(), String> {
        if next.type_name != self.type_name {
            return Err(format!(
                "type is `{}`; it cannot become `{}`",
                self.type_name, next.type_name
            ));
        }
        if next.fields.len() < self.fields.len()
            || next.fields[..self.fields.len()] != self.fields[..]
        {
            return Err(
                "existing fields cannot be changed, reordered or removed — only appended to".into(),
            );
        }
        // THE ONE THING THAT CANNOT BE EVOLVED.
        //
        // Every other change here is additive and leaves stored records where
        // they are. Declaring, removing or moving the parent changes what each
        // record's KEY is, so it is a delete-and-rewrite of every record in the
        // domain — the band migration that #117 is about. Refused, rather than
        // performed silently on the next `define`.
        if next.parent != self.parent {
            return Err(format!(
                "parent is {}; it cannot become {} — the parent decides each \
                 record's KEY, so changing it re-keys every record in the domain",
                self.parent.as_deref().unwrap_or("unset"),
                next.parent.as_deref().unwrap_or("unset"),
            ));
        }
        if let Some(f) = next.fields[self.fields.len()..].iter().find(|f| f.required) {
            return Err(format!(
                "new field `{}` cannot be required: existing records do not have it",
                f.name
            ));
        }
        Ok(())
    }
}

//! Record value encoding: flat, positional, read by offset.
//!
//! ```text
//! "R1" ‖ created:u64 ‖ updated:u64 ‖ author:[32] ‖ n:u16 ‖ offs:[u32; n+1] ‖ data
//! slot i = data[offs[i]..offs[i+1]]: empty = absent, else tag:u8 ‖ payload
//! ```
//! Slots are in schema field order, so a record written under an older schema has
//! fewer slots (the rest read as absent) and one written under a newer schema has
//! more (the extras are ignored).

use crate::schema::{Kind, Schema};
use serde_json::{Map, Value};

pub const HEADER: usize = 2 + 8 + 8 + 32 + 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub created: u64,
    pub updated: u64,
    pub author: [u8; 32],
    pub fields: Map<String, Value>,
}

fn tag(k: Kind) -> u8 {
    match k {
        Kind::Text => 1,
        Kind::Int => 2,
        Kind::Float => 3,
        Kind::Bool => 4,
        Kind::Time => 5,
        Kind::Bytes => 6,
        Kind::Ref => 7,
    }
}

fn payload(kind: Kind, v: &Value) -> Option<Vec<u8>> {
    Some(match kind {
        Kind::Text | Kind::Ref => v.as_str()?.as_bytes().to_vec(),
        Kind::Int => v.as_i64()?.to_le_bytes().to_vec(),
        Kind::Float => v.as_f64()?.to_bits().to_le_bytes().to_vec(),
        Kind::Bool => vec![v.as_bool()? as u8],
        Kind::Time => v.as_u64()?.to_le_bytes().to_vec(),
        Kind::Bytes => core_types::hex::decode(v.as_str()?)?,
    })
}

/// Validate `fields` against `schema` and encode. `null` means absent.
pub fn encode(
    schema: &Schema,
    fields: &Map<String, Value>,
    created: u64,
    updated: u64,
    author: &[u8; 32],
) -> Result<Vec<u8>, String> {
    if let Some(k) = fields
        .keys()
        .find(|k| !schema.fields.iter().any(|f| &f.name == *k))
    {
        return Err(format!("`{k}` is not a field of {}", schema.type_name));
    }
    let mut data = Vec::new();
    let mut offs = Vec::with_capacity(schema.fields.len() + 1);
    for f in &schema.fields {
        offs.push(data.len() as u32);
        match fields.get(&f.name) {
            None | Some(Value::Null) if f.required => {
                return Err(format!("`{}` is required", f.name))
            }
            None | Some(Value::Null) => {}
            Some(v) => {
                let p = payload(f.kind, v)
                    .ok_or_else(|| format!("`{}` must be {:?}", f.name, f.kind).to_lowercase())?;
                data.push(tag(f.kind));
                data.extend_from_slice(&p);
            }
        }
    }
    offs.push(data.len() as u32);
    let mut out = Vec::with_capacity(HEADER + offs.len() * 4 + data.len());
    out.extend_from_slice(b"R1");
    out.extend_from_slice(&created.to_le_bytes());
    out.extend_from_slice(&updated.to_le_bytes());
    out.extend_from_slice(author);
    out.extend_from_slice(&(schema.fields.len() as u16).to_le_bytes());
    for o in offs {
        out.extend_from_slice(&o.to_le_bytes());
    }
    out.extend_from_slice(&data);
    Ok(out)
}

/// Decode under `schema`. Never panics; malformed bytes are an error.
pub fn decode(schema: &Schema, b: &[u8]) -> Result<Decoded, String> {
    let bad = || "corrupt record".to_string();
    if b.len() < HEADER || &b[..2] != b"R1" {
        return Err(bad());
    }
    let created = u64::from_le_bytes(b[2..10].try_into().unwrap());
    let updated = u64::from_le_bytes(b[10..18].try_into().unwrap());
    let author: [u8; 32] = b[18..50].try_into().unwrap();
    let n = u16::from_le_bytes([b[50], b[51]]) as usize;
    let table_end = HEADER + (n + 1) * 4;
    if b.len() < table_end {
        return Err(bad());
    }
    let off = |i: usize| {
        u32::from_le_bytes(b[HEADER + i * 4..HEADER + i * 4 + 4].try_into().unwrap()) as usize
    };
    let data = &b[table_end..];
    let mut fields = Map::new();
    for (i, f) in schema.fields.iter().enumerate().take(n) {
        let (lo, hi) = (off(i), off(i + 1));
        if lo > hi || hi > data.len() {
            return Err(bad());
        }
        let slot = &data[lo..hi];
        let Some((&t, p)) = slot.split_first() else {
            continue;
        };
        if t != tag(f.kind) {
            return Err(bad());
        }
        let v = match f.kind {
            Kind::Text | Kind::Ref => {
                Value::String(String::from_utf8(p.to_vec()).map_err(|_| bad())?)
            }
            Kind::Int => Value::from(i64::from_le_bytes(p.try_into().map_err(|_| bad())?)),
            Kind::Float => serde_json::Number::from_f64(f64::from_bits(u64::from_le_bytes(
                p.try_into().map_err(|_| bad())?,
            )))
            .map(Value::Number)
            .ok_or_else(bad)?,
            Kind::Bool => Value::Bool(*p.first().ok_or_else(bad)? != 0),
            Kind::Time => Value::from(u64::from_le_bytes(p.try_into().map_err(|_| bad())?)),
            Kind::Bytes => Value::String(crate::hex(p)),
        };
        fields.insert(f.name.clone(), v);
    }
    Ok(Decoded {
        created,
        updated,
        author,
        fields,
    })
}

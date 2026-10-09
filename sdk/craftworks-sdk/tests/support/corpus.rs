//! The frozen corpus: blocks and head records that released contracts
//! ACCEPTED, kept so a later parser can be checked against them.
//!
//! The file format is deliberately dull — a length-prefixed list of entries,
//! each with the contract it belongs to, the epochs that accepted it, its
//! params and its state. Provenance travels WITH the bytes: a corpus whose
//! entries cannot say which epochs vouched for them is a corpus somebody has
//! to take on trust, and the whole point of it is not having to.

#![allow(dead_code)]

/// One thing a released contract said yes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `block` or `register`.
    pub contract: String,
    /// The short hashes of the epochs that accepted it, in order.
    pub epochs: Vec<String>,
    /// What this entry is a boundary of, for a failure message.
    pub what: String,
    pub params: Vec<u8>,
    pub state: Vec<u8>,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

pub fn encode(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::from(*b"CWCORPUS");
    out.extend_from_slice(&1u32.to_le_bytes()); // format version
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        put_str(&mut out, &e.contract);
        out.extend_from_slice(&(e.epochs.len() as u32).to_le_bytes());
        for ep in &e.epochs {
            put_str(&mut out, ep);
        }
        put_str(&mut out, &e.what);
        put_bytes(&mut out, &e.params);
        put_bytes(&mut out, &e.state);
    }
    out
}

/// Read the corpus back.
///
/// Every length is checked against what is actually there. A truncated file
/// must be an error and never a short list: a corpus that silently loses its
/// tail is a gate that silently checks less, and it would read as a pass.
pub fn decode(b: &[u8]) -> Result<Vec<Entry>, String> {
    let mut at = 0usize;
    let take = |at: &mut usize, n: usize| -> Result<&[u8], String> {
        let s = b
            .get(*at..*at + n)
            .ok_or_else(|| format!("corpus truncated at {at}: wanted {n} B"))?;
        *at += n;
        Ok(s)
    };
    let u32at = |at: &mut usize| -> Result<usize, String> {
        let s = take(at, 4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
    };
    if take(&mut at, 8)? != b"CWCORPUS" {
        return Err("not a corpus file".into());
    }
    let version = u32at(&mut at)?;
    if version != 1 {
        return Err(format!("corpus format version {version}, not 1"));
    }
    let n = u32at(&mut at)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let l = u32at(&mut at)?;
        let contract = String::from_utf8_lossy(take(&mut at, l)?).into_owned();
        let ne = u32at(&mut at)?;
        let mut epochs = Vec::with_capacity(ne);
        for _ in 0..ne {
            let l = u32at(&mut at)?;
            epochs.push(String::from_utf8_lossy(take(&mut at, l)?).into_owned());
        }
        let l = u32at(&mut at)?;
        let what = String::from_utf8_lossy(take(&mut at, l)?).into_owned();
        let l = u32at(&mut at)?;
        let params = take(&mut at, l)?.to_vec();
        let l = u32at(&mut at)?;
        let state = take(&mut at, l)?.to_vec();
        out.push(Entry {
            contract,
            epochs,
            what,
            params,
            state,
        });
    }
    if at != b.len() {
        return Err(format!(
            "corpus has {} trailing byte(s); it is not the file this build wrote",
            b.len() - at
        ));
    }
    Ok(out)
}

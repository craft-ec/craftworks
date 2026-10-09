//! OPERATORS (TOKEN §1b): an owner's approval of an OPERATOR for ALL its chains of one token — every item at once
//! (ERC-721's `setApprovalForAll`). One contract per (token, owner) (params [`Params`]), a record per operator key,
//! each signed by the owner's key of its key log (the DID's own key, not a device). A record keeps its SPANS — the
//! beacon intervals it was on — so a spend made while it was on stays valid after it is turned off: a chain is checked
//! from scratch long after. An operator signs spends (`Auth::Delegated`) in any of the owner's chains of the token,
//! with no limit, while its span covers the spend's stamp.

use crate::{sig_ok, Why};

pub const OPERATORS: u8 = 6;
/// The most operators one list names, and the most spans one operator keeps (turned on again past it: refused — a
/// per-item allowance still works).
pub const RECORDS_MAX: usize = 64;
pub const SPANS_MAX: usize = 8;

/// The owner's word on one OPERATOR key: the spans it was on (start, end — the last may be open), its `seq`-th word
/// (a later one replaces it), signed by the owner's key at key-log seq `key_seq`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub operator: [u8; 32],
    pub spans: Vec<(u64, Option<u64>)>,
    pub seq: u32,
    pub key_seq: u32,
    pub sig: [u8; 64],
}

impl Record {
    pub fn message(&self, p: &Params) -> Vec<u8> {
        let mut v = b"craftworks token operators".to_vec();
        v.extend_from_slice(&p.definition);
        v.extend_from_slice(&p.owner);
        self.body(&mut v);
        v
    }
    fn body(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&self.operator);
        v.push(self.spans.len() as u8);
        for (a, b) in &self.spans {
            v.extend_from_slice(&a.to_le_bytes());
            match b {
                None => v.push(0),
                Some(b) => {
                    v.push(1);
                    v.extend_from_slice(&b.to_le_bytes());
                }
            }
        }
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.key_seq.to_le_bytes());
    }
    fn encode(&self, v: &mut Vec<u8>) {
        self.body(v);
        v.extend_from_slice(&self.sig);
    }
    fn read(r: &mut crate::R) -> Option<Record> {
        let operator = r.arr()?;
        let n = r.u8()? as usize;
        (n <= SPANS_MAX).then_some(())?;
        let mut spans = Vec::with_capacity(n);
        for _ in 0..n {
            let a = r.u64()?;
            let b = match r.u8()? {
                0 => None,
                1 => Some(r.u64()?),
                _ => return None,
            };
            spans.push((a, b));
        }
        Some(Record { operator, spans, seq: r.u32()?, key_seq: r.u32()?, sig: r.arr()? })
    }
    fn hash(&self) -> [u8; 32] {
        let mut v = Vec::new();
        self.encode(&mut v);
        *blake3::hash(&v).as_bytes()
    }
    /// Was it on at beacon `stamp`?
    pub fn on_at(&self, stamp: u64) -> bool {
        self.spans.iter().any(|(a, b)| *a <= stamp && b.is_none_or(|b| stamp < b))
    }
}

/// An operators list's PARAMS: the token code's hash, the definition, the OWNER (a DID), the beacon, the key log's
/// code hash (the owner's key log is read to check its records).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub definition: [u8; 32],
    pub owner: [u8; 32],
    pub beacon: [u8; 32],
    pub idlog: [u8; 32],
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[OPERATORS][..], &self.code_hash, &self.definition, &self.owner, &self.beacon, &self.idlog].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == OPERATORS).then_some(())?;
        let p = Params { code_hash: r.arr()?, definition: r.arr()?, owner: r.arr()?, beacon: r.arr()?, idlog: r.arr()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
    /// The owner's key log: its contract id.
    pub fn keys_id(&self) -> [u8; 32] {
        crate::id(&self.idlog, &self.owner)
    }
    /// The operators list of the chain `c`'s owner and token.
    pub fn of(c: &crate::ChainParams) -> Params {
        Params { code_hash: c.code_hash, definition: c.definition, owner: c.owner, beacon: c.beacon, idlog: c.idlog }
    }
}

/// An operators list: one record per operator, sorted by operator.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct List {
    pub records: Vec<Record>,
}

impl List {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWOP".to_vec();
        v.push(self.records.len() as u8);
        for r in &self.records {
            r.encode(&mut v);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<List> {
        let mut r = crate::R(b.strip_prefix(b"CWOP")?);
        let n = r.u8()? as usize;
        (n <= RECORDS_MAX).then_some(())?;
        let records = (0..n).map(|_| Record::read(&mut r)).collect::<Option<Vec<_>>>()?;
        (r.done() && records.windows(2).all(|w| w[0].operator < w[1].operator)).then_some(List { records })
    }
    /// May `operator` sign for the owner at beacon `stamp`?
    pub fn approved(&self, operator: &[u8; 32], stamp: u64) -> bool {
        self.records.binary_search_by(|r| r.operator.cmp(operator)).is_ok_and(|i| self.records[i].on_at(stamp))
    }
}

/// Spans well formed: some, at most [`SPANS_MAX`], each closed after it opens, each after the one before; only the
/// last open.
fn spans_hold(spans: &[(u64, Option<u64>)]) -> bool {
    let n = spans.len();
    if n == 0 || n > SPANS_MAX {
        return false;
    }
    spans.iter().enumerate().all(|(i, (a, b))| match b {
        None => i == n - 1,
        Some(b) => a < b && spans.get(i + 1).is_none_or(|(next, _)| b <= next),
    })
}

/// Every record well formed and signed by the owner's key at its key seq (`log`: the owner's key log, read and
/// verified).
pub fn holds(p: &Params, log: &craftworks_idlog_contract::Log, l: &List) -> Result<(), Why> {
    for r in &l.records {
        if !spans_hold(&r.spans) {
            return Err("an operator's spans that do not follow each other");
        }
        let key = log.events.get(r.key_seq as usize).ok_or("an operator record on a key the owner never had")?.key;
        if !sig_ok(&key, &r.message(p), &r.sig) {
            return Err("an operator record not signed by the owner");
        }
    }
    Ok(())
}

/// Copies settle the same in every order: per operator, the higher seq; at one seq, the lower hash.
pub fn merge(a: &List, b: &List) -> List {
    let mut all: Vec<Record> = a.records.clone();
    for r in &b.records {
        match all.iter_mut().find(|x| x.operator == r.operator) {
            Some(x) => {
                if r.seq > x.seq || (r.seq == x.seq && r.hash() < x.hash()) {
                    *x = r.clone();
                }
            }
            None => all.push(r.clone()),
        }
    }
    all.sort_by_key(|x| x.operator);
    List { records: all }
}

/// THE APPEND RULE: what is new is signed on the owner's CURRENT key (`current`: its key log's head seq), its seq above
/// the record it replaces, and it only EXTENDS the past — every span held is kept (the last open one may be closed),
/// and every stamp it adds is within the margin of the beacon now. So no approval is back-dated and no past span is
/// taken away (a spend made under it stays valid).
pub fn fresh(held: &List, merged: &List, now: u64, margin: u64, current: u32) -> Result<(), Why> {
    let near = |x: u64| x <= now && x.saturating_add(margin) >= now;
    for r in &merged.records {
        let old = held.records.iter().find(|x| x.operator == r.operator);
        if old == Some(r) {
            continue;
        }
        if r.key_seq != current {
            return Err("an operator record not on the owner's current key");
        }
        let kept: &[(u64, Option<u64>)] = match old {
            None => &[],
            Some(o) => {
                if r.seq <= o.seq {
                    return Err("an operator record that is not newer");
                }
                &o.spans
            }
        };
        if r.spans.len() < kept.len() {
            return Err("an operator record that takes back its past");
        }
        for (i, (a, b)) in r.spans.iter().enumerate() {
            match kept.get(i) {
                Some((ka, kb)) => {
                    if ka != a {
                        return Err("an operator record that takes back its past");
                    }
                    match (kb, b) {
                        (Some(x), Some(y)) if x == y => {}
                        (None, None) => {}
                        (None, Some(y)) if near(*y) => {}
                        _ => return Err("an operator record that takes back its past"),
                    }
                }
                None => {
                    if !near(*a) || b.is_some_and(|b| !near(b)) {
                        return Err("an operator span away from the beacon");
                    }
                }
            }
        }
    }
    Ok(())
}

//! THE OUTPUTS LOG (TOKEN §5a): the beacon's state keeps only its newest steps, but a witness lot drawn long ago must
//! still be checkable (a witness checks an entry's predecessor was accepted — an account idle for days included). So
//! each FINAL step's output (past the fraud window: its points dropped) is recorded here, one contract per [`SPAN`] of
//! heights (params [`Params`]: the beacon's id and the span's index). Anyone fills it from the beacon while the beacon
//! still holds those steps: a height is taken only as the beacon holds it, and once held it never changes.

use crate::State;

pub const LOG: u8 = 0x4c;
/// Heights per log.
pub const SPAN: u64 = 4096;

/// An outputs log's PARAMS: the beacon (its contract id) and the span (`index`: heights `index·SPAN ..< (index+1)·SPAN`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub beacon: [u8; 32],
    pub index: u64,
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[LOG][..], &self.beacon, &self.index.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        if b.len() != 1 + 32 + 8 || b[0] != LOG {
            return None;
        }
        Some(Params { beacon: b[1..33].try_into().ok()?, index: u64::from_le_bytes(b[33..41].try_into().ok()?) })
    }
    /// The log a height is in.
    pub fn of(beacon: [u8; 32], height: u64) -> Params {
        Params { beacon, index: height / SPAN }
    }
    fn holds(&self, h: u64) -> bool {
        h / SPAN == self.index
    }
}

/// A log: (height, output), sorted by height, each at most once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    pub entries: Vec<(u64, [u8; 32])>,
}

impl Log {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWOL".to_vec();
        v.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (h, o) in &self.entries {
            v.extend_from_slice(&h.to_le_bytes());
            v.extend_from_slice(o);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Log> {
        let b = b.strip_prefix(b"CWOL")?;
        let n = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
        if n as u64 > SPAN || b.len() != 4 + n * 40 {
            return None;
        }
        let entries = (0..n).map(|i| {
            let x = &b[4 + i * 40..4 + (i + 1) * 40];
            (u64::from_le_bytes(x[..8].try_into().unwrap()), x[8..].try_into().unwrap())
        });
        let entries: Vec<(u64, [u8; 32])> = entries.collect();
        entries.windows(2).all(|w| w[0].0 < w[1].0).then_some(Log { entries })
    }
    /// Its heights all in its span (and in order: [`decode`](Log::decode)).
    pub fn holds(&self, p: &Params) -> bool {
        self.entries.iter().all(|(h, _)| p.holds(*h))
    }
    pub fn get(&self, h: u64) -> Option<[u8; 32]> {
        self.entries.binary_search_by_key(&h, |e| e.0).ok().map(|i| self.entries[i].1)
    }
}

/// What the beacon `b` holds FINAL (past its fraud window) in the span: what an extender records.
pub fn record(p: &Params, b: &State) -> Log {
    Log { entries: b.steps.iter().filter(|s| s.points.is_none() && p.holds(s.height)).map(|s| (s.height, s.out)).collect() }
}

/// `held` taking `given`: every height `held` lacks must be one the beacon `b` holds FINAL, with that output; a height
/// `held` has keeps its output (another one given is ignored — the beacon held it as recorded).
pub fn take(p: &Params, held: &Log, given: &Log, b: &State) -> Result<Log, &'static str> {
    if !given.holds(p) {
        return Err("an output outside the log's span");
    }
    let mut out = held.clone();
    for (h, o) in &given.entries {
        if held.get(*h).is_some() {
            continue;
        }
        match b.steps.iter().find(|s| s.height == *h) {
            Some(s) if s.points.is_none() && s.out == *o => out.entries.push((*h, *o)),
            Some(s) if s.points.is_some() => return Err("an output not final yet"),
            Some(_) => return Err("an output that is not the beacon's"),
            None => return Err("an output the beacon does not hold"),
        }
    }
    out.entries.sort_by_key(|e| e.0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params as BP;

    fn bp() -> BP {
        BP { k: 64, segs: 8, sample: 2, window: 2, keep: 5, seed: [3; 32] }
    }
    fn beacon(n: u64) -> State {
        let mut s = State::default();
        for i in 0..n {
            s = s.extend(&bp(), [i as u8; 32]);
        }
        s
    }

    #[test]
    fn a_log_takes_only_final_outputs_the_beacon_holds_and_never_changes_one() {
        let lp = Params { beacon: [9; 32], index: 0 };
        let b = beacon(7); // keeps heights 3..=7; 6 and 7 still in the fraud window
        let rec = record(&lp, &b);
        assert_eq!(rec.entries.iter().map(|e| e.0).collect::<Vec<_>>(), vec![3, 4, 5]);
        let l = take(&lp, &Log::default(), &rec, &b).unwrap();
        assert_eq!(l, rec);
        assert_eq!(Log::decode(&l.encode()), Some(l.clone()));
        assert!(Log::decode(&[&l.encode()[..], &[0]].concat()).is_none());
        // A height twice: not a log.
        let twice = Log { entries: vec![(3, [1; 32]), (3, [2; 32])] };
        assert_eq!(Log::decode(&twice.encode()), None);
        // Not final yet; another output; one the beacon no longer (or never) holds.
        let out6 = b.steps.iter().find(|s| s.height == 6).unwrap().out;
        assert_eq!(take(&lp, &l, &Log { entries: vec![(6, out6)] }, &b), Err("an output not final yet"));
        assert_eq!(take(&lp, &Log::default(), &Log { entries: vec![(4, [1; 32])] }, &b), Err("an output that is not the beacon's"));
        assert_eq!(take(&lp, &l, &Log { entries: vec![(2, [1; 32])] }, &b), Err("an output the beacon does not hold"));
        // Held: kept as recorded, whatever is given for it.
        assert_eq!(take(&lp, &l, &Log { entries: vec![(4, [1; 32])] }, &b), Ok(l.clone()));
        // Later, the beacon moved on (heights 3, 4 gone from it): the log still says them — and takes what is new.
        let later = beacon(10);
        let more = take(&lp, &l, &record(&lp, &later), &later).unwrap();
        assert_eq!(more.get(3), l.get(3));
        assert_eq!(more.entries.iter().map(|e| e.0).collect::<Vec<_>>(), vec![3, 4, 5, 6, 7, 8]);
        // Outside the span.
        let other = Params { beacon: [9; 32], index: 1 };
        assert!(record(&other, &later).entries.is_empty());
        assert_eq!(take(&other, &Log::default(), &rec, &b), Err("an output outside the log's span"));
        assert!(!rec.holds(&other));
    }
}

//! BEACON (docs/TOKEN.md §5a): the clock. A Freenet contract has no clock, so time is SEQUENTIAL WORK: step `h` is
//! `K` blake3 iterations, each on the last's output, from `start(prev out, h)` — more machines do not make it faster.
//! Anyone extends it; two extenders of one step compute the same thing.
//!
//! A step carries its `S` intermediate POINTS (the end of each of its `S` segments) while it is among the newest `F`
//! steps — the FRAUD WINDOW — and only their Merkle root after. The contract re-runs `s` segments of the newest step,
//! chosen from the step's own root (a speed bump: a cheater can grind the root). The SAFETY is the FRAUD PROOF: anyone
//! who recomputes a step and finds segment `j` wrong publishes (step, j) with the two points and their proofs; the
//! contract re-runs that one segment, and the step and all after it are gone — and stay gone (the proof is kept).
//!
//! Not yet (TOKEN §5a): the entropy MIX. This chain is deterministic from its seed, so its height is a clock but its
//! outputs are not yet randomness nobody can predict (a faster machine can compute ahead).

pub const BEACON: u8 = 0x42;
/// The most iterations one segment may hold: the cost of one sampled segment or one fraud proof inside a validation.
pub const SEG_MAX: u64 = 1 << 20;
/// Fraud proofs kept (the latest): each costs one segment to check.
pub const FRAUDS_KEPT: usize = 4;
/// Other candidate steps kept inside the fraud window (a fake and the honest step it competes with, until a proof).
pub const ALTS_KEPT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    /// Iterations per step.
    pub k: u64,
    /// Segments per step (a power of two).
    pub segs: u32,
    /// Segments re-run by the contract on the newest step.
    pub sample: u32,
    /// The fraud window: the newest `window` steps carry their points.
    pub window: u32,
    /// Steps kept in the state.
    pub keep: u32,
    pub seed: [u8; 32],
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![BEACON];
        v.extend_from_slice(&self.k.to_le_bytes());
        v.extend_from_slice(&self.segs.to_le_bytes());
        v.extend_from_slice(&self.sample.to_le_bytes());
        v.extend_from_slice(&self.window.to_le_bytes());
        v.extend_from_slice(&self.keep.to_le_bytes());
        v.extend_from_slice(&self.seed);
        v
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != 1 + 8 + 4 * 4 + 32 || b[0] != BEACON {
            return None;
        }
        let u32at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let p = Self { k: u64::from_le_bytes(b[1..9].try_into().unwrap()), segs: u32at(9), sample: u32at(13), window: u32at(17), keep: u32at(21), seed: b[25..57].try_into().unwrap() };
        p.sane().then_some(p)
    }
    fn sane(&self) -> bool {
        self.segs >= 2
            && self.segs.is_power_of_two()
            && self.segs <= 4096
            && self.k % self.segs as u64 == 0
            && self.k / self.segs as u64 >= 1
            && self.k / self.segs as u64 <= SEG_MAX
            && (1..=16).contains(&self.sample)
            && self.sample <= self.segs
            && (1..=8).contains(&self.window)
            && self.window <= self.keep
            && self.keep <= 4096
    }
    pub fn per_seg(&self) -> u64 {
        self.k / self.segs as u64
    }
    pub fn genesis(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"craftworks beacon genesis");
        h.update(&self.seed);
        *h.finalize().as_bytes()
    }
}

/// `n` sequential iterations.
pub fn seg(mut x: [u8; 32], n: u64) -> [u8; 32] {
    for _ in 0..n {
        x = *blake3::hash(&x).as_bytes();
    }
    x
}

/// Where step `height` starts.
pub fn start(prev: &[u8; 32], height: u64) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(prev);
    h.update(&height.to_le_bytes());
    *h.finalize().as_bytes()
}

fn leaf(p: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(&[0]);
    h.update(p);
    *h.finalize().as_bytes()
}
fn node(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(&[1]);
    h.update(l);
    h.update(r);
    *h.finalize().as_bytes()
}

/// The Merkle root over a step's points (their count a power of two).
pub fn root(points: &[[u8; 32]]) -> [u8; 32] {
    let mut level: Vec<[u8; 32]> = points.iter().map(leaf).collect();
    while level.len() > 1 {
        level = level.chunks(2).map(|c| node(&c[0], &c[1])).collect();
    }
    level[0]
}

/// The proof that `points[i]` is under `root(points)`.
pub fn proof(points: &[[u8; 32]], mut i: usize) -> Vec<[u8; 32]> {
    let mut level: Vec<[u8; 32]> = points.iter().map(leaf).collect();
    let mut out = Vec::new();
    while level.len() > 1 {
        out.push(level[i ^ 1]);
        level = level.chunks(2).map(|c| node(&c[0], &c[1])).collect();
        i /= 2;
    }
    out
}

pub fn verify(root_: &[u8; 32], mut i: usize, p: &[u8; 32], path: &[[u8; 32]]) -> bool {
    let mut x = leaf(p);
    for s in path {
        x = if i % 2 == 0 { node(&x, s) } else { node(s, &x) };
        i /= 2;
    }
    &x == root_
}

/// The segments the contract re-runs, chosen from the step itself.
pub fn samples(p: &Params, root_: &[u8; 32], prev: &[u8; 32], height: u64) -> Vec<usize> {
    (0..p.sample)
        .map(|i| {
            let mut h = blake3::Hasher::new();
            h.update(root_);
            h.update(prev);
            h.update(&height.to_le_bytes());
            h.update(&i.to_le_bytes());
            u32::from_le_bytes(h.finalize().as_bytes()[..4].try_into().unwrap()) as usize % p.segs as usize
        })
        .collect()
}

/// One STEP. `points` while in the fraud window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub height: u64,
    pub prev: [u8; 32],
    pub out: [u8; 32],
    pub root: [u8; 32],
    pub points: Option<Vec<[u8; 32]>>,
}

impl Step {
    /// COMPUTE step `height` after `prev` — the extender's work (`K` sequential iterations).
    pub fn compute(p: &Params, prev: [u8; 32], height: u64) -> Self {
        let mut x = start(&prev, height);
        let mut points = Vec::with_capacity(p.segs as usize);
        for _ in 0..p.segs {
            x = seg(x, p.per_seg());
            points.push(x);
        }
        Self { height, prev, out: x, root: root(&points), points: Some(points) }
    }
    /// The segment `j`'s two ends, as this step claims them.
    pub fn ends(&self, j: usize) -> Option<([u8; 32], [u8; 32])> {
        let pts = self.points.as_ref()?;
        let a = if j == 0 { start(&self.prev, self.height) } else { pts[j - 1] };
        Some((a, pts[j]))
    }
    /// Find a wrong segment by recomputing every one (a verifier's work: in parallel, in practice).
    pub fn find_fraud(&self, p: &Params) -> Option<Fraud> {
        let pts = self.points.as_ref()?;
        (0..pts.len()).find_map(|j| {
            let (a, b) = self.ends(j)?;
            (seg(a, p.per_seg()) != b).then(|| Fraud::of(self, j))
        })
    }
}

/// A FRAUD PROOF: segment `j` of the step at `height` (with root `root`) does not compute — standalone, checkable
/// with the step gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fraud {
    pub height: u64,
    pub prev: [u8; 32],
    pub root: [u8; 32],
    pub j: u32,
    pub a: [u8; 32],
    pub a_path: Vec<[u8; 32]>,
    pub b: [u8; 32],
    pub b_path: Vec<[u8; 32]>,
}

impl Fraud {
    pub fn of(s: &Step, j: usize) -> Self {
        let pts = s.points.as_ref().expect("points");
        let (a, b) = s.ends(j).unwrap();
        Self { height: s.height, prev: s.prev, root: s.root, j: j as u32, a, a_path: if j == 0 { vec![] } else { proof(pts, j - 1) }, b, b_path: proof(pts, j) }
    }
    /// It proves what it says: both ends under the root, and the segment between them does not compute.
    pub fn holds(&self, p: &Params) -> bool {
        let j = self.j as usize;
        if j >= p.segs as usize {
            return false;
        }
        let a_ok = if j == 0 { self.a == start(&self.prev, self.height) } else { verify(&self.root, j - 1, &self.a, &self.a_path) };
        a_ok && verify(&self.root, j, &self.b, &self.b_path) && seg(self.a, p.per_seg()) != self.b
    }
    fn refutes(&self, s: &Step) -> bool {
        s.height == self.height && s.root == self.root
    }
}

/// The beacon's STATE: its newest steps (heights in a row), and the fraud proofs that keep fakes out.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct State {
    /// The CANONICAL chain: the longest the steps known allow.
    pub steps: Vec<Step>,
    /// Other candidates inside the fraud window — kept, so a fraud proof that removes a fake brings back what it hid.
    pub alts: Vec<Step>,
    pub frauds: Vec<Fraud>,
}

impl State {
    pub fn height(&self) -> u64 {
        self.steps.last().map_or(0, |s| s.height)
    }
    pub fn out(&self, p: &Params) -> [u8; 32] {
        self.steps.last().map_or(p.genesis(), |s| s.out)
    }
    /// The next step, computed and added (an extender's whole job).
    pub fn extend(&self, p: &Params) -> State {
        let mut next = self.clone();
        next.steps.push(Step::compute(p, self.out(p), self.height() + 1));
        normalize(p, next)
    }
}

/// Why a state is refused.
pub type Why = &'static str;

/// THE RULES of a beacon state.
pub fn check(p: &Params, s: &State) -> Result<(), Why> {
    if s.steps.len() > p.keep as usize {
        return Err("more steps than it keeps");
    }
    if s.frauds.len() > FRAUDS_KEPT {
        return Err("more fraud proofs than it keeps");
    }
    for f in &s.frauds {
        if !f.holds(p) {
            return Err("a fraud proof that does not prove fraud");
        }
    }
    let window_from = s.steps.len().saturating_sub(p.window as usize);
    for (i, st) in s.steps.iter().enumerate() {
        if i == 0 {
            if st.height == 1 && st.prev != p.genesis() {
                return Err("a first step not after the genesis");
            }
        } else {
            let before = &s.steps[i - 1];
            if st.height != before.height + 1 || st.prev != before.out {
                return Err("a broken link");
            }
        }
        if s.frauds.iter().any(|f| f.refutes(st)) {
            return Err("a step proven fraudulent");
        }
        match (&st.points, i >= window_from) {
            (Some(pts), true) => {
                if pts.len() != p.segs as usize || root(pts) != st.root || pts.last() != Some(&st.out) {
                    return Err("points that are not the step's");
                }
            }
            (None, false) => {}
            (Some(_), false) => return Err("points kept past the fraud window"),
            (None, true) => return Err("a step in the fraud window without its points"),
        }
    }
    if s.alts.len() > ALTS_KEPT {
        return Err("more candidates than it keeps");
    }
    for a in &s.alts {
        let low = s.steps.first().map_or(0, |x| x.height);
        let pts = a.points.as_ref().ok_or("a candidate without its points")?;
        if a.height < low || pts.len() != p.segs as usize || root(pts) != a.root || pts.last() != Some(&a.out) {
            return Err("a candidate that is not a step at or above the chain's start");
        }
        if s.frauds.iter().any(|f| f.refutes(a)) || s.steps.iter().any(|st| st.height == a.height && st.root == a.root) {
            return Err("a candidate proven fraudulent or already in the chain");
        }
    }
    // The newest step: its sampled segments re-run (the speed bump).
    if let Some(top) = s.steps.last() {
        for j in samples(p, &top.root, &top.prev, top.height) {
            let (a, b) = top.ends(j).ok_or("a step in the fraud window without its points")?;
            if seg(a, p.per_seg()) != b {
                return Err("a sampled segment that does not compute");
            }
        }
    }
    Ok(())
}

/// Steps kept, points only in the window, fraud proofs the latest few: the one shape every copy agrees on.
pub fn normalize(p: &Params, mut s: State) -> State {
    let over = s.steps.len().saturating_sub(p.keep as usize);
    s.steps.drain(..over);
    let window_from = s.steps.len().saturating_sub(p.window as usize);
    for st in &mut s.steps[..window_from] {
        st.points = None;
    }
    s.frauds.sort_by_key(|f| (f.height, f.root, f.j));
    s.frauds.dedup_by_key(|f| (f.height, f.root));
    let n = s.frauds.len();
    s.frauds.drain(..n.saturating_sub(FRAUDS_KEPT));
    let canon: Vec<(u64, [u8; 32])> = s.steps.iter().map(|x| (x.height, x.root)).collect();
    // Candidates kept by a rule nobody running ahead can move: the LOWEST heights first (a fake chain racing ahead
    // cannot push out the honest step it competes with) — above the chain's top too: a step whose parent is not
    // known yet (removed with a fake it shared an output with, or still on its way) waits for it.
    let low = s.steps.first().map_or(0, |x| x.height);
    s.alts.retain(|a| a.points.is_some() && a.height >= low && !canon.contains(&(a.height, a.root)));
    s.alts.sort_by_key(|a| (a.height, a.out, a.root));
    s.alts.dedup_by_key(|a| (a.height, a.root));
    s.alts.truncate(ALTS_KEPT);
    s
}

/// THE MERGE: the same result in any order — computed from the UNION of every step both copies know (their chains and
/// their candidates), never by discarding one copy: fraud proofs from both remove what they refute; the CANONICAL
/// chain is the longest path the rest allows (where two paths are equally long, the lower output where they first
/// differ: one of two different steps at a height is a fake — the chain is deterministic — and stays only until a
/// proof); the rest stay as candidates while in the fraud window, so a proof that removes a fake brings back the
/// honest step it hid.
pub fn merge(p: &Params, a: &State, b: &State) -> State {
    let mut frauds: Vec<Fraud> = a.frauds.iter().chain(&b.frauds).cloned().collect();
    frauds.sort_by_key(|f| (f.height, f.root, f.j));
    frauds.dedup_by_key(|f| (f.height, f.root));
    // Every step known, once (with its points where any copy has them), none proven fraudulent.
    let mut pool: Vec<Step> = Vec::new();
    for st in a.steps.iter().chain(&a.alts).chain(&b.steps).chain(&b.alts) {
        if frauds.iter().any(|f| f.refutes(st)) {
            continue;
        }
        match pool.iter_mut().find(|x| x.height == st.height && x.root == st.root) {
            Some(x) => {
                if x.points.is_none() {
                    x.points = st.points.clone();
                }
            }
            None => pool.push(st.clone()),
        }
    }
    pool.sort_by_key(|x| (x.height, x.out, x.root));
    // The longest path through the pool (heights in a row, each step's prev the last's out); ties: the lower output
    // at the first difference. Paths start at the lowest height known.
    let start = pool.first().map_or(0, |x| x.height);
    // Iteratively (a beacon keeps 1440 steps: no recursion that deep in a contract): from the highest height down,
    // each step's longest path onward = 1 + its longest child's (pool sorted by height, then output, so on equal
    // length the first — lowest output — child is kept).
    let n = pool.len();
    let mut len = vec![1usize; n];
    let mut next: Vec<Option<usize>> = vec![None; n];
    for i in (0..n).rev() {
        for j in (i + 1)..n {
            if pool[j].height > pool[i].height + 1 {
                break;
            }
            if pool[j].height == pool[i].height + 1 && pool[j].prev == pool[i].out && 1 + len[j] > len[i] {
                len[i] = 1 + len[j];
                next[i] = Some(j);
            }
        }
    }
    let mut steps: Vec<Step> = Vec::new();
    if let Some(first) = (0..n).filter(|&i| pool[i].height == start).fold(None, |best: Option<usize>, i| match best {
        Some(b) if len[b] >= len[i] => Some(b),
        _ => Some(i),
    }) {
        let mut at = Some(first);
        while let Some(i) = at {
            steps.push(pool[i].clone());
            at = next[i];
        }
    }
    let alts: Vec<Step> = pool.into_iter().filter(|x| !steps.iter().any(|c| c.height == x.height && c.root == x.root)).collect();
    normalize(p, State { steps, alts, frauds })
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::{check, merge, Params};
    use freenet_stdlib::prelude::*;

    pub struct Beacon;

    #[contract]
    impl ContractInterface for Beacon {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, _related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            let Some(p) = Params::decode(parameters.as_ref()) else { return Ok(ValidateResult::Invalid) };
            let Some(s) = super::wire::decode(state.as_ref()) else { return Ok(ValidateResult::Invalid) };
            Ok(if check(&p, &s).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            let p = Params::decode(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
            let mut held = super::wire::decode(state.as_ref()).ok_or(ContractError::InvalidState)?;
            for d in data {
                let b: &[u8] = match &d {
                    UpdateData::State(s) => s.as_ref(),
                    UpdateData::Delta(x) => x.as_ref(),
                    UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                    _ => continue,
                };
                let given = super::wire::decode(b).ok_or(ContractError::InvalidUpdate)?;
                held = merge(&p, &held, &given);
            }
            Ok(UpdateModification::valid(State::from(super::wire::encode(&held))))
        }

        fn summarize_state(_parameters: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            Ok(StateSummary::from(blake3::hash(state.as_ref()).as_bytes().to_vec()))
        }

        fn get_state_delta(_parameters: Parameters<'static>, state: State<'static>, summary: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
            Ok(StateDelta::from(if summary.as_ref() == blake3::hash(state.as_ref()).as_bytes() { Vec::new() } else { state.as_ref().to_vec() }))
        }
    }
}

/// The state's bytes.
pub mod wire {
    use super::{Fraud, State, Step};

    struct R<'a>(&'a [u8]);
    impl<'a> R<'a> {
        fn take(&mut self, n: usize) -> Option<&'a [u8]> {
            if self.0.len() < n {
                return None;
            }
            let (a, b) = self.0.split_at(n);
            self.0 = b;
            Some(a)
        }
        fn h(&mut self) -> Option<[u8; 32]> {
            self.take(32)?.try_into().ok()
        }
        fn u32(&mut self) -> Option<u32> {
            Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
        }
        fn u64(&mut self) -> Option<u64> {
            Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
        }
        fn hs(&mut self, limit: usize) -> Option<Vec<[u8; 32]>> {
            let n = self.u32()? as usize;
            if n > limit {
                return None;
            }
            (0..n).map(|_| self.h()).collect()
        }
    }
    fn hs(v: &mut Vec<u8>, xs: &[[u8; 32]]) {
        v.extend_from_slice(&(xs.len() as u32).to_le_bytes());
        for x in xs {
            v.extend_from_slice(x);
        }
    }

    fn step(v: &mut Vec<u8>, st: &Step) {
        v.extend_from_slice(&st.height.to_le_bytes());
        v.extend_from_slice(&st.prev);
        v.extend_from_slice(&st.out);
        v.extend_from_slice(&st.root);
        match &st.points {
            None => v.push(0),
            Some(p) => {
                v.push(1);
                hs(v, p);
            }
        }
    }

    pub fn encode(s: &State) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(s.steps.len() as u32).to_le_bytes());
        for st in &s.steps {
            step(&mut v, st);
        }
        v.extend_from_slice(&(s.alts.len() as u32).to_le_bytes());
        for st in &s.alts {
            step(&mut v, st);
        }
        v.extend_from_slice(&(s.frauds.len() as u32).to_le_bytes());
        for f in &s.frauds {
            v.extend_from_slice(&f.height.to_le_bytes());
            v.extend_from_slice(&f.prev);
            v.extend_from_slice(&f.root);
            v.extend_from_slice(&f.j.to_le_bytes());
            v.extend_from_slice(&f.a);
            hs(&mut v, &f.a_path);
            v.extend_from_slice(&f.b);
            hs(&mut v, &f.b_path);
        }
        v
    }

    pub fn decode(b: &[u8]) -> Option<State> {
        if b.is_empty() {
            return Some(State::default());
        }
        let mut r = R(b);
        let n = r.u32()? as usize;
        if n > 4096 {
            return None;
        }
        let read = |r: &mut R| -> Option<Step> {
            let height = r.u64()?;
            let prev = r.h()?;
            let out = r.h()?;
            let root = r.h()?;
            let points = match r.take(1)?[0] {
                0 => None,
                1 => Some(r.hs(4096)?),
                _ => return None,
            };
            Some(Step { height, prev, out, root, points })
        };
        let mut steps = Vec::with_capacity(n);
        for _ in 0..n {
            steps.push(read(&mut r)?);
        }
        let k = r.u32()? as usize;
        if k > 64 {
            return None;
        }
        let mut alts = Vec::with_capacity(k);
        for _ in 0..k {
            alts.push(read(&mut r)?);
        }
        let m = r.u32()? as usize;
        if m > 64 {
            return None;
        }
        let mut frauds = Vec::with_capacity(m);
        for _ in 0..m {
            frauds.push(Fraud { height: r.u64()?, prev: r.h()?, root: r.h()?, j: r.u32()?, a: r.h()?, a_path: r.hs(16)?, b: r.h()?, b_path: r.hs(16)? });
        }
        r.0.is_empty().then_some(State { steps, alts, frauds })
    }
}

#[cfg(test)]
mod tests;

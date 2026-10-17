//! WITNESSED COMMITS (the proof before any product code — owner 10-17: "forks should not happen"; TOKEN §3, §7a): a
//! group's commits are POSITIONS (its epoch), like a token chain's entries; a member APPLIES a commit only once it is
//! FINAL — signed by a QUORUM of witnesses, each of whom signs at most ONE commit per position. Two quorums always share
//! a witness, so (with fewer dishonest witnesses than the overlap) at most one commit per position is ever final: no
//! member ever applies a commit another member does not — no fork, nothing to heal. Delay is accepted (propagation and
//! offline periods are the network's), forks are not.
//!
//! THE SIMULATION (real MLS — this crate's `SpaceMember` / `Account` — over a simulated network):
//! - 5 members (M0 made the group), 4 witnesses (quorum 3: one dishonest witness tolerated — it signs everything).
//! - Every message (a candidate commit, a signature, a group info) reaches each actor after its own random delay; actors
//!   go offline and come back; a member may LOSE its state (it enters again by itself).
//! - The LIST (who belongs) changes at random: someone added, someone taken out. A listed member without keys ENTERS
//!   (an external commit from the newest final group info it sees); a member that sees an unlisted entry proposes its
//!   REMOVAL. Several may propose for the same position at once — that is the race the old design forked on.
//! - A proposer whose candidate did not become final drops it (its group as it was) and tries again later.
//!
//! CHECKED, every step: (1) at most one final commit per position; (2) any two members at the same epoch hold the same
//! epoch secret. CHECKED at the end, after a quiet period (no list changes, everyone online, all delivered): (3) every
//! listed member holds the newest epoch; (4) the group's entries are exactly the list; (5) no removed member holds it.
//! CONTROL: quorum 2 with the dishonest witness — two quorums can then share only the liar — must produce a detected
//! fork in some run (the checker can see one).
use craftworks_mls::{Account, Rule, SpaceMember};
use ed25519_dalek::SigningKey;
use std::collections::{BTreeMap, BTreeSet};

const SPACE: [u8; 32] = [0x77; 32];
const MEMBERS: usize = 5;
const WITNESSES: usize = 4;
const TICKS: u64 = 400;
const QUIET: u64 = 200;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, per_mille: u64) -> bool {
        self.below(1000) < per_mille
    }
}

fn space_member(i: usize) -> (SpaceMember, String) {
    let (did, data) = ([0x10 + i as u8; 32], [0x40 + i as u8; 32]);
    let seed = craftworks_identity::space_member_seed(&data);
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let m = SpaceMember::new(&seed, craftworks_identity::space_member_credential(&did, &data, &public)).unwrap();
    (m, craftworks_account::did(&did))
}

/// Something sent, and when each actor sees it (`MEMBERS + WITNESSES` actors).
struct Sent<T> {
    what: T,
    seen_at: Vec<u64>,
}

/// A candidate commit for a position.
struct Candidate {
    epoch: u64,
    by: usize,
    commit: Vec<u8>,
}

struct Member {
    m: SpaceMember,
    did: String,
    /// Its group (applied: final commits only), as saved.
    held: Option<Vec<u8>>,
    /// Its proposal in flight: (candidate index, its group after it).
    pending: Option<(usize, Vec<u8>)>,
    offline_until: u64,
}

struct World {
    rng: Rng,
    now: u64,
    members: Vec<Member>,
    listed: BTreeSet<usize>,
    candidates: Vec<Sent<Candidate>>,
    /// (candidate, witness) signatures.
    sigs: Vec<Sent<(usize, usize)>>,
    /// Each witness's one signed candidate per position (an honest one's guard).
    signed: Vec<BTreeMap<u64, usize>>,
    /// Final group infos: (epoch, info).
    infos: Vec<Sent<(u64, Vec<u8>)>>,
    quorum: usize,
    liar: Option<usize>,
    /// What each position became final as, by anyone's view (the check: never two).
    finals: BTreeMap<u64, BTreeSet<usize>>,
}

impl World {
    fn new(seed: u64, quorum: usize, liar: Option<usize>) -> World {
        let mut members: Vec<Member> = (0..MEMBERS).map(|i| { let (m, did) = space_member(i); Member { m, did, held: None, pending: None, offline_until: 0 } }).collect();
        let mut g = members[0].m.create_space(SPACE).unwrap();
        let info = g.group_info().unwrap();
        members[0].held = Some(g.save().unwrap());
        let mut w = World { rng: Rng(seed | 1), now: 0, members, listed: [0].into(), candidates: Vec::new(), sigs: Vec::new(), signed: vec![BTreeMap::new(); WITNESSES], infos: Vec::new(), quorum, liar, finals: BTreeMap::new() };
        w.send_info(0, info);
        w
    }
    fn delays(&mut self) -> Vec<u64> {
        let now = self.now;
        (0..MEMBERS + WITNESSES).map(|_| now + 1 + self.rng.below(12)).collect()
    }
    fn send_info(&mut self, epoch: u64, info: Vec<u8>) {
        let seen_at = self.delays();
        self.infos.push(Sent { what: (epoch, info), seen_at });
    }
    fn load(&self, i: usize) -> Option<Account> {
        self.members[i].held.as_ref().map(|b| Account::load(Rule::Space(SPACE), b).unwrap())
    }
    /// Candidate `c` is final in actor `a`'s view (a quorum of signatures it has seen).
    fn final_for(&self, a: usize, c: usize) -> bool {
        let ws: BTreeSet<usize> = self.sigs.iter().filter(|s| s.what.0 == c && s.seen_at[a] <= self.now).map(|s| s.what.1).collect();
        ws.len() >= self.quorum
    }

    fn step(&mut self, quiet: bool) {
        self.now += 1;
        // THE LIST changes (an owner's act): someone added, someone taken out (never M0, the owner).
        if !quiet && self.rng.chance(40) {
            let i = 1 + self.rng.below(MEMBERS as u64 - 1) as usize;
            if !self.listed.insert(i) {
                self.listed.remove(&i);
            }
        }
        // WITNESSES: each signs the first candidate it sees for a position (the liar signs every one).
        for w in 0..WITNESSES {
            let actor = MEMBERS + w;
            for c in 0..self.candidates.len() {
                if self.candidates[c].seen_at[actor] > self.now || self.sigs.iter().any(|s| s.what == (c, w)) {
                    continue;
                }
                let e = self.candidates[c].what.epoch;
                let honest_ok = !self.signed[w].contains_key(&e);
                if Some(w) == self.liar || honest_ok {
                    self.signed[w].entry(e).or_insert(c);
                    let seen_at = self.delays();
                    self.sigs.push(Sent { what: (c, w), seen_at });
                }
            }
        }
        // MEMBERS, in a random order.
        let mut order: Vec<usize> = (0..MEMBERS).collect();
        for k in (1..order.len()).rev() {
            let j = self.rng.below(k as u64 + 1) as usize;
            order.swap(k, j);
        }
        for i in order {
            if !quiet && self.rng.chance(15) {
                self.members[i].offline_until = self.now + 1 + self.rng.below(40);
            }
            if self.members[i].offline_until > self.now {
                continue;
            }
            // A device LOSES its state now and then (it enters again by itself).
            if !quiet && i != 0 && self.members[i].held.is_some() && self.rng.chance(5) {
                self.members[i].held = None;
                self.members[i].pending = None;
            }
            self.member_acts(i);
        }
        self.check_agreement();
    }

    fn member_acts(&mut self, i: usize) {
        // 1. APPLY every final commit at its position, in order (its own: its pending state taken).
        loop {
            let Some(mut g) = self.load(i) else { break };
            let e = g.epoch();
            let fin: Vec<usize> = (0..self.candidates.len()).filter(|&c| self.candidates[c].what.epoch == e && self.candidates[c].seen_at[i] <= self.now && self.final_for(i, c)).collect();
            let Some(&c) = fin.first() else { break };
            self.finals.entry(e).or_default().extend(fin.iter().copied());
            let mine = self.members[i].pending.as_ref().filter(|(pc, _)| *pc == c).map(|(_, st)| st.clone());
            if let Some(st) = mine {
                self.members[i].held = Some(st);
                self.members[i].pending = None;
            } else {
                match g.process(&self.candidates[c].what.commit) {
                    Ok(()) => {
                        // Taken out by it: it holds nothing from here on.
                        self.members[i].held = if g.members().is_empty() { None } else { Some(g.save().unwrap()) };
                    }
                    Err(_) => {
                        // This device was removed by the commit (MLS: a removed member cannot go on).
                        self.members[i].held = None;
                    }
                }
                // Its own proposal lost the position: dropped.
                if self.members[i].pending.as_ref().is_some_and(|(pc, _)| self.candidates[*pc].what.epoch <= e) {
                    self.members[i].pending = None;
                }
            }
            if let Some(mut g2) = self.load(i) {
                let info = g2.group_info().unwrap();
                let ep = g2.epoch();
                self.send_info(ep, info);
            }
        }
        // AN ENTRY in flight (no group held yet): its position decided — this entry final (its group taken), another
        // final (dropped: it enters again from a newer info) — else wait.
        if self.members[i].held.is_none() {
            if let Some((pc, st)) = self.members[i].pending.clone() {
                let e = self.candidates[pc].what.epoch;
                let fin: Vec<usize> = (0..self.candidates.len()).filter(|&c| self.candidates[c].what.epoch == e && self.candidates[c].seen_at[i] <= self.now && self.final_for(i, c)).collect();
                if let Some(&c) = fin.first() {
                    self.finals.entry(e).or_default().extend(fin.iter().copied());
                    self.members[i].pending = None;
                    if c == pc {
                        self.members[i].held = Some(st);
                        let mut g = self.load(i).unwrap();
                        let info = g.group_info().unwrap();
                        let ep = g.epoch();
                        self.send_info(ep, info);
                    }
                }
            }
        }
        // A proposal still in flight: wait for it (or for another to win its position).
        if self.members[i].pending.is_some() {
            return;
        }
        let listed = self.listed.contains(&i);
        match self.load(i) {
            // 2. LISTED, NO KEYS: enter by itself from the newest final group info it sees.
            None if listed => {
                let newest = self.infos.iter().filter(|s| s.seen_at[i] <= self.now).max_by_key(|s| s.what.0).map(|s| s.what.clone());
                if let Some((e, info)) = newest {
                    if let Ok((mut g, commit)) = self.members[i].m.enter(SPACE, &info, b"") {
                        let st = g.save().unwrap();
                        self.propose(i, e, commit, Some(st));
                    }
                }
            }
            // 3. HOLDING KEYS: propose the removal of every entry the list no longer names (not its own).
            Some(mut g) if listed => {
                let gone: Vec<u32> = g.entries().into_iter().filter(|(_, d, _)| !self.members.iter().enumerate().any(|(k, m)| m.did == *d && self.listed.contains(&k))).map(|(ix, _, _)| ix).collect();
                if !gone.is_empty() && self.rng.chance(300) {
                    let e = g.epoch();
                    let before = g.save().unwrap();
                    if let Ok(commit) = g.remove(&gone) {
                        let st = g.save().unwrap();
                        self.members[i].held = Some(before);
                        self.propose(i, e, commit, Some(st));
                    }
                }
            }
            _ => {}
        }
    }

    fn propose(&mut self, i: usize, epoch: u64, commit: Vec<u8>, after: Option<Vec<u8>>) {
        let seen_at = self.delays();
        self.candidates.push(Sent { what: Candidate { epoch, by: i, commit }, seen_at });
        if let Some(st) = after {
            self.members[i].pending = Some((self.candidates.len() - 1, st));
        }
    }

    /// (1) at most one final commit per position; (2) members at one epoch hold one secret.
    fn check_agreement(&mut self) -> Option<String> {
        for (e, cs) in &self.finals {
            if cs.len() > 1 {
                return Some(format!("FORK at epoch {e}: {} final commits", cs.len()));
            }
        }
        let mut by_epoch: BTreeMap<u64, [u8; 32]> = BTreeMap::new();
        for i in 0..MEMBERS {
            if let Some(g) = self.load(i) {
                let s = g.epoch_secret().unwrap();
                if let Some(other) = by_epoch.insert(g.epoch(), s) {
                    if other != s {
                        return Some(format!("FORK: two secrets at epoch {}", g.epoch()));
                    }
                }
            }
        }
        None
    }
}

/// One run: `Ok(newest epoch)` or the first broken property.
fn run(seed: u64, quorum: usize, liar: Option<usize>) -> Result<u64, String> {
    let mut w = World::new(seed, quorum, liar);
    for t in 0..TICKS + QUIET {
        let quiet = t >= TICKS;
        if quiet {
            for m in &mut w.members {
                m.offline_until = 0;
            }
        }
        w.step(quiet);
        if let Some(f) = w.check_agreement() {
            return Err(format!("seed {seed} tick {t}: {f}"));
        }
    }
    // (3)–(5) after the quiet period.
    let newest = (0..MEMBERS).filter_map(|i| w.load(i).map(|g| g.epoch())).max().unwrap_or(0);
    // A position with candidates and none final (what a failure below would be): each candidate's signatures.
    let stuck = |w: &World| -> String {
        let mut by: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (c, x) in w.candidates.iter().enumerate() {
            if !w.finals.contains_key(&x.what.epoch) {
                by.entry(x.what.epoch).or_default().push(w.sigs.iter().filter(|s| s.what.0 == c).count());
            }
        }
        format!("undecided positions (signatures per candidate): {by:?}")
    };
    for i in 0..MEMBERS {
        let held = w.load(i).map(|g| g.epoch());
        if w.listed.contains(&i) && held != Some(newest) {
            return Err(format!("seed {seed}: M{i} listed but holds {held:?}, newest {newest}; {}", stuck(&w)));
        }
        if !w.listed.contains(&i) && held == Some(newest) {
            return Err(format!("seed {seed}: M{i} taken out but holds the newest epoch {newest}; {}", stuck(&w)));
        }
    }
    let g = w.load(0).unwrap();
    let entries: BTreeSet<String> = g.entries().into_iter().map(|(_, d, _)| d).collect();
    let want: BTreeSet<String> = w.listed.iter().map(|&i| w.members[i].did.clone()).collect();
    if entries != want {
        return Err(format!("seed {seed}: the group holds {} entries, the list names {}", entries.len(), want.len()));
    }
    Ok(newest)
}

#[test]
#[ignore = "evidence for the BeeKEM proposal: run with --ignored (minutes)"]
fn witnessed_commits_never_fork_and_every_listed_member_ends_current() {
    let runs: u64 = std::env::var("SIM_RUNS").ok().and_then(|x| x.parse().ok()).unwrap_or(200);
    let mut epochs = 0;
    let mut failed = Vec::new();
    for seed in 1..=runs {
        match run(seed * 7919, 3, Some(0)) {
            Ok(e) => epochs += e,
            Err(f) => failed.push(f),
        }
    }
    println!("witnessed: {runs} runs, {} failed, {epochs} epochs made in all", failed.len());
    for f in failed.iter().take(10) {
        println!("  {f}");
    }
    assert!(failed.is_empty(), "{} of {runs} runs broke a property", failed.len());
}

#[test]
#[ignore = "evidence for the BeeKEM proposal: run with --ignored (minutes)"]
fn control_a_quorum_that_can_share_only_the_liar_forks() {
    let runs: u64 = std::env::var("SIM_RUNS").ok().and_then(|x| x.parse().ok()).unwrap_or(200);
    let forks = (1..=runs).filter(|s| run(s * 7919, 2, Some(0)).is_err_and(|e| e.contains("FORK"))).count();
    println!("control (quorum 2, one liar): {forks} of {runs} runs forked");
    assert!(forks > 0, "the checker must see a fork when quorums need not overlap in an honest witness");
}

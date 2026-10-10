//! A peer's STANDING RECORD (TOKEN §7a): per epoch, its attestations, bindings, holdings and answers — the state of its
//! standing contract. Everything is checked FROM SCRATCH against the beacon's outputs ([`holds`]): a node makes its
//! own contract first, so nothing in it is taken on trust. What falls out of the beacon's window is pruned.

use crate::{check, place, sampled, Answer, Attest, Bind, Holdings, Why, EPOCH};

/// At least this many distinct attesters place a peer.
pub const QUORUM: usize = 3;
pub const ATTESTS_MAX: usize = 32;
pub const BINDS_MAX: usize = 4;
pub const HOLDINGS_MAX: usize = 4;
/// Answers kept per epoch (each holdings' sampled heights: ~EPOCH / SAMPLE of them).
pub const ANSWERS_MAX: usize = 4 * 16;
/// Epochs kept (the beacon keeps at most 4096 steps: 4 epochs, and the one running).
pub const EPOCHS_MAX: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Epoch {
    pub epoch: u64,
    pub attests: Vec<Attest>,
    pub binds: Vec<Bind>,
    pub holdings: Vec<Holdings>,
    pub answers: Vec<Answer>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub epochs: Vec<Epoch>,
}

/// Every record of peer `peer`'s state holds: its own peer and epoch; signed; holdings only by a key bound in that
/// epoch; every answer by those holdings' key, at a height the beacon still keeps (`out`) and sampled for this peer,
/// the entry the beacon asked for, under the committed root, in the arc its attesters give it.
pub fn holds(peer: &str, s: &State, out: &dyn Fn(u64) -> Option<[u8; 32]>) -> Result<(), Why> {
    let mut last = None;
    for e in &s.epochs {
        if last.is_some_and(|l| e.epoch <= l) {
            return Err("epochs out of order");
        }
        last = Some(e.epoch);
        if e.attests.len() > ATTESTS_MAX || e.binds.len() > BINDS_MAX || e.holdings.len() > HOLDINGS_MAX || e.answers.len() > ANSWERS_MAX {
            return Err("an epoch over its caps");
        }
        if e.attests.iter().any(|a| a.peer != peer || a.epoch != e.epoch || !a.holds()) {
            return Err("an attestation that does not hold here");
        }
        if e.binds.iter().any(|b| b.peer != peer || b.epoch != e.epoch || !b.holds()) {
            return Err("a binding that does not hold here");
        }
        let bound: Vec<[u8; 32]> = e.binds.iter().map(|b| b.key).collect();
        if e.holdings.iter().any(|h| h.peer != peer || h.epoch != e.epoch || !bound.contains(&h.key) || !h.holds()) {
            return Err("holdings not by a key bound to the peer");
        }
        if e.answers.is_empty() {
            continue;
        }
        // LOCATION a soft rule (attestations are sybil-able; the seal is the guard): the arc applies when the peer is placed.
        let arc = place(peer, e.epoch, &e.attests, &bound, QUORUM).map(|(_, arc)| arc);
        for a in &e.answers {
            let h = e.holdings.iter().find(|h| h.key == a.key).ok_or("an answer for no holdings")?;
            let o = out(a.height).ok_or("an answer outside the beacon's window")?;
            if !sampled(&o, peer) {
                return Err("an answer at a height not sampled for this peer");
            }
            check(h, a, &o, arc, None)?;
        }
    }
    Ok(())
}

fn dedup<T: PartialEq + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let mut v = a.to_vec();
    for x in b {
        if !v.contains(x) {
            v.push(x.clone());
        }
    }
    v
}

/// The MERGE: per epoch the union of both copies' records, in one order (each kept to its cap); the newest
/// [`EPOCHS_MAX`] epochs.
pub fn merge(a: &State, b: &State) -> State {
    let mut epochs: Vec<Epoch> = a.epochs.clone();
    for e in &b.epochs {
        match epochs.iter_mut().find(|x| x.epoch == e.epoch) {
            Some(x) => {
                x.attests = dedup(&x.attests, &e.attests);
                x.binds = dedup(&x.binds, &e.binds);
                x.holdings = dedup(&x.holdings, &e.holdings);
                x.answers = dedup(&x.answers, &e.answers);
            }
            None => epochs.push(e.clone()),
        }
    }
    epochs.sort_by_key(|e| e.epoch);
    let drop = epochs.len().saturating_sub(EPOCHS_MAX);
    epochs.drain(..drop);
    // ONE ORDER whichever copy merged into which (every copy converges on the same bytes): each list by its records'
    // signatures (answers by height first); the caps then keep the same records everywhere.
    for e in &mut epochs {
        e.attests.sort_by(|x, y| x.sig.cmp(&y.sig));
        e.binds.sort_by(|x, y| x.sig.cmp(&y.sig));
        e.holdings.sort_by(|x, y| x.sig.cmp(&y.sig));
        e.answers.sort_by(|x, y| (x.height, x.sig).cmp(&(y.height, y.sig)));
        e.attests.truncate(ATTESTS_MAX);
        e.binds.truncate(BINDS_MAX);
        e.holdings.truncate(HOLDINGS_MAX);
        e.answers.truncate(ANSWERS_MAX);
    }
    State { epochs }
}

/// What falls out of the beacon's window: epochs ending below its oldest kept height.
pub fn prune(s: &State, oldest_height: u64) -> State {
    State { epochs: s.epochs.iter().filter(|e| (e.epoch + 1) * EPOCH > oldest_height).cloned().collect() }
}

/// Distinct sampled heights a keeper must have ANSWERED in an epoch to have KEPT in it (of ~EPOCH / SAMPLE = 8 sampled:
/// half — a page open part of the epoch).
pub const KEEP_MIN: usize = 4;

/// Whether key `key` KEPT in standing epoch `epoch` by its record (committed holdings and at least [`KEEP_MIN`]
/// answered heights): the statement's `kept`. Assumes [`holds`] (the standing contract keeps only records that do).
pub fn kept(s: &State, key: &[u8; 32], epoch: u64) -> bool {
    s.epochs.iter().find(|e| e.epoch == epoch).is_some_and(|e| {
        e.holdings.iter().any(|h| h.key == *key) && e.answers.iter().filter(|a| a.key == *key).map(|a| a.height).collect::<std::collections::BTreeSet<_>>().len() >= KEEP_MIN
    })
}

/// The ACCOUNTS that kept in `epoch`: of the keepers listed `(account, record key)`, those whose record
/// (`record_of(key)`: `None` — not there) [`kept`]; each account once, in order.
pub fn kept_accounts(listed: &[([u8; 32], [u8; 32])], record_of: &dyn Fn(&[u8; 32]) -> Option<State>, epoch: u64) -> Vec<[u8; 32]> {
    let mut out: Vec<[u8; 32]> = listed.iter().filter(|(_, key)| record_of(key).is_some_and(|s| kept(&s, key, epoch))).map(|x| x.0).collect();
    out.sort();
    out.dedup();
    out
}

/// STANDING: the bytes proved held — each holdings' bytes × the share of its peer's sampled heights (the beacon's, up
/// to `now`) it answered. `out`: the beacon's outputs. Assumes [`holds`].
pub fn weight(peer: &str, s: &State, out: &dyn Fn(u64) -> Option<[u8; 32]>, now: u64) -> u128 {
    let mut w = 0u128;
    for e in &s.epochs {
        let (from, to) = (e.epoch * EPOCH, ((e.epoch + 1) * EPOCH).min(now + 1));
        let due = (from..to).filter(|h| out(*h).is_some_and(|o| sampled(&o, peer))).count() as u128;
        if due == 0 {
            continue;
        }
        for h in &e.holdings {
            let answered = e.answers.iter().filter(|a| a.key == h.key).map(|a| a.height).collect::<std::collections::BTreeSet<_>>().len() as u128;
            w += h.bytes as u128 * answered.min(due) / due;
        }
    }
    w
}

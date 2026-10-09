//! The standing record's BYTES: `CWST ‖ n (u8) ‖ n × epoch`, each epoch its records counted and in order. Exact: a
//! decode reads every byte, refuses a count over its cap, a peer over [`PEER_MAX`].

use crate::record::{Epoch, State, ANSWERS_MAX, ATTESTS_MAX, BINDS_MAX, EPOCHS_MAX, HOLDINGS_MAX};
use crate::{Answer, Attest, Bind, Holdings, PEER_MAX};

struct R<'a>(&'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        (self.0.len() >= n).then(|| {
            let (a, b) = self.0.split_at(n);
            self.0 = b;
            a
        })
    }
    fn arr<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.arr()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.arr()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.arr()?))
    }
    fn peer(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        (n <= PEER_MAX).then_some(())?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    fn count(&mut self, max: usize) -> Option<usize> {
        let n = self.u16()? as usize;
        (n <= max).then_some(n)
    }
}

fn peer(v: &mut Vec<u8>, p: &str) {
    v.push(p.len() as u8);
    v.extend_from_slice(p.as_bytes());
}

pub fn encode(s: &State) -> Vec<u8> {
    let mut v = b"CWST".to_vec();
    v.push(s.epochs.len() as u8);
    for e in &s.epochs {
        v.extend_from_slice(&e.epoch.to_le_bytes());
        v.extend_from_slice(&(e.attests.len() as u16).to_le_bytes());
        for a in &e.attests {
            v.extend_from_slice(&a.by);
            peer(&mut v, &a.peer);
            v.extend_from_slice(&a.ring.to_le_bytes());
            v.extend_from_slice(&a.at.to_le_bytes());
            v.extend_from_slice(&a.epoch.to_le_bytes());
            v.extend_from_slice(&a.sig);
        }
        v.extend_from_slice(&(e.binds.len() as u16).to_le_bytes());
        for b in &e.binds {
            v.extend_from_slice(&b.key);
            peer(&mut v, &b.peer);
            v.extend_from_slice(&b.epoch.to_le_bytes());
            v.extend_from_slice(&b.sig);
        }
        v.extend_from_slice(&(e.holdings.len() as u16).to_le_bytes());
        for h in &e.holdings {
            v.extend_from_slice(&h.key);
            peer(&mut v, &h.peer);
            v.extend_from_slice(&h.epoch.to_le_bytes());
            v.extend_from_slice(&h.root);
            v.extend_from_slice(&h.count.to_le_bytes());
            v.extend_from_slice(&h.bytes.to_le_bytes());
            v.extend_from_slice(&h.sig);
        }
        v.extend_from_slice(&(e.answers.len() as u16).to_le_bytes());
        for a in &e.answers {
            v.extend_from_slice(&a.key);
            peer(&mut v, &a.peer);
            v.extend_from_slice(&a.height.to_le_bytes());
            v.extend_from_slice(&a.index.to_le_bytes());
            v.extend_from_slice(&a.id);
            v.push(a.path.len() as u8);
            for p in &a.path {
                match p {
                    None => v.push(0),
                    Some(s) => {
                        v.push(1);
                        v.extend_from_slice(s);
                    }
                }
            }
            v.extend_from_slice(&a.hash);
            v.extend_from_slice(&a.sig);
        }
    }
    v
}

pub fn decode(b: &[u8]) -> Option<State> {
    let mut r = R(b.strip_prefix(b"CWST")?);
    let n = r.u8()? as usize;
    (n <= EPOCHS_MAX).then_some(())?;
    let mut epochs = Vec::with_capacity(n);
    for _ in 0..n {
        let epoch = r.u64()?;
        let attests = (0..r.count(ATTESTS_MAX)?).map(|_| Some(Attest { by: r.arr()?, peer: r.peer()?, ring: r.u64()?, at: r.u64()?, epoch: r.u64()?, sig: r.arr()? })).collect::<Option<Vec<_>>>()?;
        let binds = (0..r.count(BINDS_MAX)?).map(|_| Some(Bind { key: r.arr()?, peer: r.peer()?, epoch: r.u64()?, sig: r.arr()? })).collect::<Option<Vec<_>>>()?;
        let holdings = (0..r.count(HOLDINGS_MAX)?)
            .map(|_| Some(Holdings { key: r.arr()?, peer: r.peer()?, epoch: r.u64()?, root: r.arr()?, count: r.u32()?, bytes: r.u64()?, sig: r.arr()? }))
            .collect::<Option<Vec<_>>>()?;
        let answers = (0..r.count(ANSWERS_MAX)?)
            .map(|_| {
                let (key, peer, height, index, id) = (r.arr()?, r.peer()?, r.u64()?, r.u32()?, r.arr()?);
                let n = r.u8()? as usize;
                (n <= 64).then_some(())?;
                let path = (0..n).map(|_| match r.u8()? { 0 => Some(None), 1 => Some(Some(r.arr()?)), _ => None }).collect::<Option<Vec<_>>>()?;
                Some(Answer { key, peer, height, index, id, path, hash: r.arr()?, sig: r.arr()? })
            })
            .collect::<Option<Vec<_>>>()?;
        epochs.push(Epoch { epoch, attests, binds, holdings, answers });
    }
    r.0.is_empty().then_some(State { epochs })
}

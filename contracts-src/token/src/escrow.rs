//! ESCROW (TOKEN §5): the token's ONE door for apps. An owner locks an amount of a chain's token for an outside
//! contract — a PROGRAM (a swap, a pool, anything) — with an `Escrow` entry; it comes back only as that program's
//! published VERDICT says: an `Unescrow` in the escrow's own chain (its refund) and a `Payout` in each recipient's chain.
//! The token knows nothing of what the program does.
//!
//! Every program's state BEGINS with a [`Section`] in one standard encoding (what follows is its own): the escrows it
//! holds and its verdict. The rule every reader can check of ANY program ([`Section::holds`]): no escrow listed twice,
//! no refund past its escrow, and in each token the refunds and payouts within what was escrowed — so a program, even
//! one that lies, only ever redistributes what was escrowed with it, never makes money. Witnesses check besides that
//! every listed escrow is a real, ACCEPTED escrow to that program (`wallet::program_backed`).

use crate::{Tok, Why};

/// An escrow's place: the token of its chain, its owner, its nonce.
pub type EscrowRef = (Tok, [u8; 32], [u8; 16]);

/// The most escrows, refunds and payouts one program lists, and the longest terms an escrow carries.
pub const LIST_MAX: usize = 4096;
pub const TERMS_MAX: usize = 256;

/// The hash of an escrow's TERMS (what a program lists it by).
pub fn terms_hash(terms: &[u8]) -> [u8; 32] {
    *blake3::Hasher::new_derive_key("craftworks token escrow terms").update(terms).finalize().as_bytes()
}

/// A program's standard SECTION.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    /// The escrows it holds, each with its amount and the hash of its TERMS ([`terms_hash`]) — both as its `Escrow` entry
    /// says (witnesses check it): a program cannot list anyone's escrow on terms they did not sign.
    pub escrows: Vec<(EscrowRef, u128, [u8; 32])>,
    /// What goes back to an escrow's owner (in the escrow's own chain).
    pub refunds: Vec<(EscrowRef, u128)>,
    /// What is paid out: (its key — once each —, the token, the recipient, the amount).
    pub pays: Vec<([u8; 16], Tok, [u8; 32], u128)>,
}

fn put_ref(v: &mut Vec<u8>, r: &EscrowRef) {
    r.0.put(v);
    v.extend_from_slice(&r.1);
    v.extend_from_slice(&r.2);
}
fn read_ref(r: &mut crate::R) -> Option<EscrowRef> {
    Some((Tok::read(r)?, r.arr()?, r.arr()?))
}

impl Section {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWPG".to_vec();
        v.extend_from_slice(&(self.escrows.len() as u32).to_le_bytes());
        for (r, a, t) in &self.escrows {
            put_ref(&mut v, r);
            v.extend_from_slice(&a.to_le_bytes());
            v.extend_from_slice(t);
        }
        v.extend_from_slice(&(self.refunds.len() as u32).to_le_bytes());
        for (r, a) in &self.refunds {
            put_ref(&mut v, r);
            v.extend_from_slice(&a.to_le_bytes());
        }
        v.extend_from_slice(&(self.pays.len() as u32).to_le_bytes());
        for (k, t, to, a) in &self.pays {
            v.extend_from_slice(k);
            t.put(&mut v);
            v.extend_from_slice(to);
            v.extend_from_slice(&a.to_le_bytes());
        }
        v
    }
    /// The section a program's state begins with, and the rest (the program's own bytes).
    pub fn decode_prefix(b: &[u8]) -> Option<(Section, &[u8])> {
        let mut r = crate::R(b.strip_prefix(b"CWPG")?);
        let count = |r: &mut crate::R| r.u32().map(|n| n as usize).filter(|n| *n <= LIST_MAX);
        let n = count(&mut r)?;
        let escrows = (0..n).map(|_| Some((read_ref(&mut r)?, r.u128()?, r.arr()?))).collect::<Option<Vec<_>>>()?;
        let n = count(&mut r)?;
        let refunds = (0..n).map(|_| Some((read_ref(&mut r)?, r.u128()?))).collect::<Option<Vec<_>>>()?;
        let n = count(&mut r)?;
        let pays = (0..n).map(|_| Some((r.arr()?, Tok::read(&mut r)?, r.arr()?, r.u128()?))).collect::<Option<Vec<_>>>()?;
        Some((Section { escrows, refunds, pays }, r.0))
    }
    /// THE RULE of any program: each escrow listed once; each refund for a listed escrow, once, within its amount;
    /// each payout key once; and per token, refunds and payouts within what was escrowed.
    pub fn holds(&self) -> Result<(), Why> {
        let mut seen: Vec<&EscrowRef> = Vec::new();
        for (r, ..) in &self.escrows {
            if seen.contains(&r) {
                return Err("an escrow listed twice");
            }
            seen.push(r);
        }
        let mut refunded: Vec<&EscrowRef> = Vec::new();
        for (r, a) in &self.refunds {
            let held = self.escrows.iter().find(|(x, ..)| x == r).ok_or("a refund of an escrow not held")?.1;
            if *a > held || refunded.contains(&r) {
                return Err("a refund past its escrow");
            }
            refunded.push(r);
        }
        let mut keys: Vec<&[u8; 16]> = Vec::new();
        for (k, ..) in &self.pays {
            if keys.contains(&k) {
                return Err("a payout key used twice");
            }
            keys.push(k);
        }
        let mut toks: Vec<Tok> = self.escrows.iter().map(|((t, ..), ..)| *t).chain(self.pays.iter().map(|(_, t, ..)| *t)).collect();
        toks.sort();
        toks.dedup();
        for t in toks {
            let sum = |it: &mut dyn Iterator<Item = u128>| -> Option<u128> {
                let mut s = 0u128;
                for a in it {
                    s = s.checked_add(a)?;
                }
                Some(s)
            };
            let held = sum(&mut self.escrows.iter().filter(|((x, ..), ..)| *x == t).map(|(_, a, _)| *a)).ok_or("an overflow")?;
            let back = sum(&mut self.refunds.iter().filter(|((x, ..), _)| *x == t).map(|(_, a)| *a)).ok_or("an overflow")?;
            let out = sum(&mut self.pays.iter().filter(|(_, x, ..)| *x == t).map(|(.., a)| *a)).ok_or("an overflow")?;
            if back.checked_add(out).is_none_or(|x| x > held) {
                return Err("a program paying out more than was escrowed with it");
            }
        }
        Ok(())
    }
    /// The refund the verdict gives an escrow (`None`: none yet).
    pub fn refund_of(&self, r: &EscrowRef) -> Option<u128> {
        self.refunds.iter().find(|(x, _)| x == r).map(|(_, a)| *a)
    }
    /// The payout `key`: (its token, its recipient, its amount).
    pub fn pay(&self, key: &[u8; 16]) -> Option<(Tok, [u8; 32], u128)> {
        self.pays.iter().find(|(k, ..)| k == key).map(|(_, t, to, a)| (*t, *to, *a))
    }
}

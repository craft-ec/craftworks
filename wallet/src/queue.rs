//! THE WITNESS QUEUE (TOKEN §7a): how an entry reaches its witnesses. Its writer — online as it writes — drops a
//! [`Queued`] item (the chain and the place) into the epoch's queue, a chain of bags ([`crate::chained`]); a witness
//! reads the queue, draws each entry's committee, and — seated — re-checks it by induction (`wallet::may_witness`) and
//! signs into the entry's witness bag (`wallet::witness_bag`). Nothing in it names an account the chain does not.

use craftworks_token_contract::Tok;

/// One entry to witness: the chain (its token, its owner) and the entry's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Queued {
    pub token: Tok,
    pub owner: [u8; 32],
    pub seq: u64,
}

const QUEUED: &[u8; 4] = b"CWWQ";

impl Queued {
    pub fn encode(&self) -> Vec<u8> {
        [&QUEUED[..], &self.token.definition, &self.token.item, &self.owner, &self.seq.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Queued> {
        if b.len() != 4 + 32 * 3 + 8 || &b[..4] != QUEUED {
            return None;
        }
        let a = |i: usize| -> [u8; 32] { b[i..i + 32].try_into().unwrap() };
        Some(Queued { token: Tok { definition: a(4), item: a(36) }, owner: a(68), seq: u64::from_le_bytes(b[100..108].try_into().unwrap()) })
    }
}

/// Queue bag `k` of standing epoch `epoch`.
pub fn address(epoch: u64, k: u32) -> [u8; 32] {
    crate::chained::address("craftworks 2026-10-10 witness queue", epoch, k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_queued_entry_reads_back_and_queues_differ_from_payments() {
        let q = Queued { token: Tok { definition: [1; 32], item: [2; 32] }, owner: [3; 32], seq: 9 };
        assert_eq!(Queued::decode(&q.encode()), Some(q));
        assert_eq!(Queued::decode(&q.encode()[1..]), None);
        assert_ne!(address(5, 0), address(5, 1));
        assert_ne!(address(5, 0), crate::stealth::payments_address(5, 0));
    }
}

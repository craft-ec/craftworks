//! THE FILES REGISTRY (REWARDS §4, owner 10-10): every Craftworks file the network should keep, listed per standing
//! epoch in a chain of public bags ([`crate::chained`]) — its ADDRESS KEY (`craftworks_files::address_key`: every
//! fragment's address, nothing it opens) and its SIZE (its plan). A keeper derives the pieces from them; anyone sums the
//! sizes (the network's volume). Nothing names the uploader. An entry lives the epoch it is listed in and the next: its
//! holders list it again each epoch, so a file nobody keeps drops out. An entry is UNSIGNED — anyone can list an
//! address key nothing serves — so what a keeper spends its limit on is decided by its own reads
//! (`craftworks_standing::assign::fill`, `craftworks_files::found`), never by a listing alone.

/// One file listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listed {
    pub address_key: [u8; 32],
    pub size: u64,
}

const LISTED: &[u8; 4] = b"CWFL";

impl Listed {
    pub fn encode(&self) -> Vec<u8> {
        [&LISTED[..], &self.address_key, &self.size.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Listed> {
        if b.len() != 4 + 32 + 8 || &b[..4] != LISTED {
            return None;
        }
        Some(Listed { address_key: b[4..36].try_into().ok()?, size: u64::from_le_bytes(b[36..44].try_into().ok()?) })
    }
}

/// Registry bag `k` of standing epoch `epoch`.
pub fn address(epoch: u64, k: u32) -> [u8; 32] {
    crate::chained::address("craftworks 2026-10-10 files registry", epoch, k)
}

/// The files LIVE at standing epoch `epoch` from the items of its registry and the one before (each file once: the
/// same address key listed twice is one file).
pub fn live(this: &[Vec<u8>], before: &[Vec<u8>]) -> Vec<Listed> {
    let mut out: Vec<Listed> = this.iter().chain(before).filter_map(|b| Listed::decode(b)).collect();
    out.sort_by_key(|l| l.address_key);
    out.dedup_by_key(|l| l.address_key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_reads_back_and_live_files_are_counted_once() {
        let a = Listed { address_key: [1; 32], size: 300 };
        let b = Listed { address_key: [2; 32], size: 70 };
        assert_eq!(Listed::decode(&a.encode()), Some(a));
        assert_eq!(Listed::decode(&a.encode()[1..]), None);
        let live = live(&[a.encode(), b.encode()], &[a.encode(), b"junk".to_vec()]);
        assert_eq!(live, vec![a, b]);
        assert_eq!(live.iter().map(|l| l.size).sum::<u64>(), 370);
        assert_ne!(address(5, 0), crate::queue::address(5, 0));
    }
}

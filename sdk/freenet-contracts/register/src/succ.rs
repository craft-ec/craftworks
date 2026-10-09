//! Naming a successor, for writers and readers — never for the contract.
//!
//! A terminal record says "this register moved to that one". The value it
//! carries must survive a code upgrade, because the record is signed and
//! immutable: whatever it names is named for ever.
//!
//! A Freenet instance id cannot do that. It is `BLAKE3(code hash ‖ params)`, so
//! it names the CODE as much as the register, and the day this contract is
//! upgraded every terminal record ever written would point under a dead build.
//!
//! The identity here names the register and nothing else. A reader turns it
//! into a current key through the epoch table; the table is what knows about
//! code, and it is the only thing that should.
//!
//! **Compiled out of the contract's wasm.** `validate_state` checks the LENGTH
//! of a terminal value and never its meaning, so a host needs none of this —
//! and `register.wasm`'s hash is part of every register's key, so it must not
//! move for a helper no host calls.

use crate::wire::HASH_LEN;

/// Domain separator for a successor's identity. Distinct from the signature and
/// state prefixes so an identity can never be read as either.
pub const SUCC_DOMAIN: &[u8; 9] = b"RG01-succ";

/// The value a terminal record carries to name a successor:
/// `BLAKE3("RG01-succ" ‖ successor params)`, over the successor's PARAMETER
/// BYTES as they appear in its key.
pub fn successor_value(successor_params: &[u8]) -> [u8; HASH_LEN] {
    let mut h = blake3::Hasher::new();
    h.update(SUCC_DOMAIN);
    h.update(successor_params);
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identity is the successor's parameters and nothing else — recomputed
    /// here independently, so anything else folded in would show.
    #[test]
    fn a_successor_is_named_by_identity_not_by_code() {
        let params = b"RG01\x00\x01\x02\x03successor-params";
        let got = successor_value(params);

        let mut h = blake3::Hasher::new();
        h.update(SUCC_DOMAIN);
        h.update(params);
        assert_eq!(
            got,
            *h.finalize().as_bytes(),
            "the preimage is not domain ‖ params"
        );

        // Frozen: a "moved-to" written today must still resolve tomorrow, so
        // this value may never move. No code hash is an input, so no upgrade
        // can move it.
        assert_eq!(
            got.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "d551459860520ef7df1624a5c3589e36f8ee335a6c684c491177a360e246723e"
        );

        // A different successor is a different name; the same one is the same.
        assert_ne!(got, successor_value(b"RG01\x00\x01\x02\x03other-params"));
        assert_eq!(got, successor_value(params));
        assert_eq!(got.len(), crate::wire::TERMINAL_VALUE_LEN);
    }
}

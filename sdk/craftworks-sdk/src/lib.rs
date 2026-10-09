//! App-facing SDK (ARCHITECTURE.md §19). Grows one surface per build phase:
//! `collection` (1) · `identity` `query` `subscribe` (3) · `inbox` `edge`
//! `stream` (6) · `cap` (8) · `file` `blob` (9) · `ledger` (10).
//!
//! Every capability is written once in Rust and exposed to JavaScript from
//! [`js`], so the builder and apps get it the phase it lands.

pub mod app;
pub mod blockid;
pub mod db;
pub mod definition;
pub mod engine_client;
pub mod expected;
pub mod id;
pub mod live_bindings;
pub mod page_store;
pub mod read_token;
pub mod record;
pub mod schema;
pub mod status;
pub mod store;
pub mod tick_gate;
pub mod trace;
pub mod tree_store;
pub mod writes;

/// What this build IS, baked from the source by `build.rs`.
///
/// Consts rather than `env!` at each use site, because `build.rs` runs for
/// THIS crate: a second crate reaching for the same variable finds nothing,
/// and the browser build is a second crate now. One source, re-exported.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The contract hashes this build provisions with — COPIED from the contracts
/// build, never re-hashed. See `build.rs`.
pub const BLOCK_HASH: &str = env!("SDK_BLOCK_HASH");
/// The `webapp` contract code this build ships (`hashes.toml`, `sha256:<hex>`): the only code a load piece is PUT under
/// (`put_piece`, app-as-data P5 -- never a caller's code).
pub const WEBAPP_HASH: &str = env!("SDK_WEBAPP_HASH");
pub const REGISTER_HASH: &str = env!("SDK_REGISTER_HASH");
pub const CONTRACTS_REV: &str = env!("SDK_CONTRACTS_REV");

pub use blockid::{BlockId, ContentHash, IdError};
pub use writes::Writes;
pub use db::{CreateAt, Db, DbError, Record, RerunEvent, RerunStep, Scan, SLOT_SKEW_MS};
pub use id::slot_from;
pub use engine_client::Client;
pub use freenet_prolly::Cid;
pub use id::{Env, RKey, SystemEnv};
pub use live_bindings::LiveBindings;
pub use page_store::{Ended, Outcome, PageStore};
pub use schema::{Field, Kind, Schema};
pub use store::{Delta, Edit, IdWidth, Read, Reads, Refused, Store, StoreError};
pub use trace::{Stamped, Trace, Traces};
pub use tree_store::{Stats, TreeStore};

/// The tree format tag every node carries, from the library that writes them.
///
/// Read from `freenet_prolly` rather than written down here. A tag this crate
/// spelled itself would keep reporting `PT01` the day the library moved on,
/// which is precisely the drift a version panel exists to catch.
pub fn format_tag() -> &'static str {
    // `MAGIC` is four ASCII bytes by construction, asserted below.
    std::str::from_utf8(freenet_prolly::node::MAGIC).expect("the format tag is ASCII")
}

/// The commit this build came from, as `build.rs` stamped it. `unknown` when
/// nothing could say — which a consumer must treat as a mismatch, not as an
/// absence.
pub const BUILD_REV: &str = env!("SDK_BUILD_REV");

/// The `freenet-prolly` revision this build LINKS, from `Cargo.lock`.
pub const PROLLY_REV: &str = env!("SDK_PROLLY_REV");

/// Lower-case hex of `bytes` (layer 0's one encoder).
pub use core_types::hex::encode as hex;

// `cid` and `cid_hex` were here, returning `BLAKE3(bytes)` under a name that
// reads like an address. Nothing in this crate called them — the tree makes its
// own ids through `freenet_prolly::block_id` — so the only thing they did was
// offer apps a 32-byte value that looks like a block id and fetches nothing.
// They are now [`ContentHash`] and [`BlockId`], which are different types.

#[cfg(test)]
mod tests {
    use super::{BlockId, ContentHash};

    fn bytes(input: &str) -> Vec<u8> {
        core_types::hex::decode(input).unwrap()
    }

    /// The same vectors are checked from JavaScript (`tests/js/ids.test.cjs`), so
    /// a binding that mangles bytes on the way in or out cannot pass both.
    ///
    /// The file is unchanged by this rename: these are the same numbers they
    /// always were, now under the name that says what they are. The block ids
    /// beside them are new, and are what apps actually get.
    #[test]
    fn the_frozen_vectors_hold_in_rust() {
        let mut n = 0;
        for line in include_str!("../tests/vectors.txt").lines() {
            let (input, want) = line.split_once(' ').unwrap();
            assert_eq!(
                super::hex(&ContentHash::of(&bytes(input)).bytes()),
                want,
                "input {input}"
            );
            n += 1;
        }
        assert!(n >= 4, "only {n} vectors");

        let mut m = 0;
        for line in include_str!("../tests/block_vectors.txt").lines() {
            let mut it = line.split(' ');
            let (tag, input, want) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
            let b = bytes(input);
            let id = match tag {
                "raw" => BlockId::raw(&b),
                "node" => BlockId::node(&b),
                t => panic!("unknown tag {t}"),
            };
            assert_eq!(id.to_string(), want, "input {input}");
            m += 1;
        }
        assert!(m >= 4, "only {m} block vectors");
    }
}

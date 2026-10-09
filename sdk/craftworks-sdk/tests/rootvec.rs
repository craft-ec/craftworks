//! A root hash for a FIXED dataset, pinned.
//!
//! Nothing in this repo pinned a tree root, so a change to the tree library
//! moved every root silently. That is exactly what a vector is for: the root is
//! the whole contents in 32 bytes, and a library change that moves it changes
//! what every existing reader is looking for.
//!
//! When this fails, it is not a number to update. It says the tree format
//! moved, and whoever moved it owes an answer about the data already written
//! under the old one.

use craftworks_sdk::store::Store;
use craftworks_sdk::TreeStore;

/// Deterministic, small, and spanning both inline and referenced values.
fn fixture() -> TreeStore {
    let mut s = TreeStore::new();
    for i in 0..500u64 {
        // A repeating pattern rather than randomness: the fixture has to be the
        // same tomorrow, in another language, on another machine.
        let n = 16 + (i * 37 % 97) as usize;
        s.put(format!("k/{i:06}").as_bytes(), &vec![(i % 251) as u8; n]).expect("the store took the write");
    }
    // Two values well over MAX_INLINE, so referenced values are in the picture.
    s.put(b"big/a", &vec![7u8; 4096]).expect("the store took the write");
    s.put(b"big/b", &vec![9u8; 9000]).expect("the store took the write");
    s
}

#[test]
fn the_fixed_dataset_still_has_the_root_it_had() {
    let root = craftworks_sdk::hex(&fixture().root());
    println!("fixture root = {root}");
    assert_eq!(
        root,
        include_str!("root_vector.txt").trim(),
        "the tree format moved: every root written under the old format now names \
         something no reader will find. This is not a number to update on its own."
    );
}

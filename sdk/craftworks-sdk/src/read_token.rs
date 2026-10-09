//! The token a write's READ is compared by (M2, sdk#148): a value's hash in
//! its STORED form — inline bytes, or a reference to the block that holds it —
//! exactly as the engine compares it (`engine::leaf_hash`). The core does not
//! link the engine, so this is computed here from the tree library's own
//! `Value::for_bytes` (the format's one decision of inline vs reference), and
//! `tests/read_token.rs` pins it to the engine's on every stored-form shape.

/// The `Expect::Value` token for a value the writer READ as `bytes`.
pub fn read_token(bytes: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    match freenet_prolly::node::Value::for_bytes(bytes).0 {
        freenet_prolly::node::Value::Inline(b) => {
            h.update(&[0]);
            h.update(b);
        }
        freenet_prolly::node::Value::Ref { cid, len } => {
            h.update(&[1]);
            h.update(&cid);
            h.update(&len.to_le_bytes());
        }
    }
    *h.finalize().as_bytes()
}

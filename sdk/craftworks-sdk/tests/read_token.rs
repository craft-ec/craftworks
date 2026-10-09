//! The client's read token IS the engine's: pinned on every stored form
//! (inline, and by reference past the inline limit), at the boundary too.

#[test]
fn the_clients_read_token_is_the_engines_leaf_hash() {
    let mut sizes: Vec<usize> = vec![0, 1, 31, 32, 33, 255, 256, 1000, 4096, 20_000];
    // Around whatever size the format stops inlining at.
    sizes.extend((1..=5000).step_by(97));
    let mut forms = std::collections::BTreeSet::new();
    for n in sizes {
        let bytes: Vec<u8> = (0..n).map(|i| (i * 7 + n) as u8).collect();
        assert_eq!(craftworks_sdk::read_token::read_token(&bytes), engine::leaf_hash(&bytes), "{n} bytes");
        forms.insert(matches!(freenet_prolly::node::Value::for_bytes(&bytes).0, freenet_prolly::node::Value::Inline(_)));
    }
    assert_eq!(forms.len(), 2, "every value took ONE stored form: the pin cannot tell inline from by-reference");
}

//! Telling a block id from a content hash, in Rust and in text.

use craftworks_sdk::blockid::tag_of;
use craftworks_sdk::{BlockId, ContentHash, IdError};
use freenet_prolly::kind;
use std::str::FromStr;

/// The reason these are two types: the SAME bytes have two different ids, and
/// only one of them fetches anything.
#[test]
fn the_same_bytes_have_two_different_ids() {
    let b = b"value";
    let id = BlockId::raw(b);
    let h = ContentHash::of(b);
    assert_ne!(id.hash(), h.bytes(), "the whole issue: these must differ");
    assert!(id.to_string().starts_with("raw:2585d5bd"), "{id}");
    // Not 6437b3ac…, which is what issue #7 gives for these bytes: that is the
    // frozen vector for `abc` (tests/vectors.txt line 2), picked up by mistake.
    // The block-id half of the issue's example is right.
    assert!(h.to_string().starts_with("hash:06444a49"), "{h}");
    assert_eq!(
        ContentHash::of(b"abc").to_string(),
        "hash:6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
        "where the issue's 6437b3ac actually comes from"
    );

    // And the block id is the substrate's, not a second implementation of it.
    assert_eq!(id.hash(), freenet_prolly::block_id(kind::RAW, b));
    assert_eq!(
        BlockId::node(b).hash(),
        freenet_prolly::block_id(kind::TREE_NODE, b)
    );
    // A content hash is BLAKE3 of the bytes alone.
    assert_eq!(h.bytes(), *blake3::hash(b).as_bytes());
}

#[test]
fn an_id_survives_the_round_trip_and_has_one_spelling() {
    for b in [&b""[..], b"x", b"value", &[0xff; 300]] {
        for id in [BlockId::raw(b), BlockId::node(b)] {
            let s = id.to_string();
            assert_eq!(BlockId::from_str(&s), Ok(id), "{s}");
            assert_eq!(s, format!("{id:?}"), "Debug must be the same text");
            assert_eq!(s.to_lowercase(), s, "ids are lower case");
            // Upper-case hex is a DIFFERENT string for the same id, so it is
            // refused rather than accepted: two spellings of one id would make
            // string comparison of ids unsafe.
            assert_eq!(
                BlockId::from_str(&s.to_uppercase()),
                Err(IdError::Malformed)
            );
        }
        let h = ContentHash::of(b);
        assert_eq!(ContentHash::from_str(&h.to_string()), Ok(h));
    }
}

/// The mistake this exists to stop: one sort of id where the other belongs.
/// It must be a parse error naming both, not a lookup that misses.
#[test]
fn each_type_refuses_the_other_and_says_which_is_which() {
    let id = BlockId::raw(b"value");
    let h = ContentHash::of(b"value");

    let e = BlockId::from_str(&h.to_string()).unwrap_err();
    assert_eq!(
        e,
        IdError::WrongSort {
            want: "block id",
            got: "content hash"
        }
    );
    assert_eq!(
        e.to_string(),
        "this is a content hash, and a block id is needed here"
    );

    let e = ContentHash::from_str(&id.to_string()).unwrap_err();
    assert_eq!(
        e,
        IdError::WrongSort {
            want: "content hash",
            got: "block id"
        }
    );

    // A bare 64-hex string is what every id looked like before, so it gets its
    // own error rather than "unknown tag".
    let bare = craftworks_sdk::hex(&id.hash());
    assert_eq!(BlockId::from_str(&bare), Err(IdError::Untagged));
    assert_eq!(ContentHash::from_str(&bare), Err(IdError::Untagged));
    assert!(IdError::Untagged.to_string().contains("no tag"));
}

#[test]
fn nothing_else_parses() {
    let hex64 = craftworks_sdk::hex(&BlockId::raw(b"x").hash());
    for (what, s) in [
        ("empty", String::new()),
        ("a tag alone", "raw:".into()),
        ("hex alone with a colon", format!(":{hex64}")),
        ("short hex", format!("raw:{}", &hex64[..63])),
        ("long hex", format!("raw:{hex64}0")),
        ("not hex", format!("raw:{}z", &hex64[..63])),
        ("upper-case hex", format!("raw:{}", hex64.to_uppercase())),
        ("two colons", format!("raw:node:{hex64}")),
        ("whitespace", format!(" raw:{hex64}")),
        ("trailing space", format!("raw:{hex64} ")),
        ("a record id", "0199c2f4a1b2000041424344000000ff".into()),
        ("a url", format!("https://x/raw:{hex64}")),
    ] {
        assert!(BlockId::from_str(&s).is_err(), "{what} parsed: {s:?}");
        assert!(ContentHash::from_str(&s).is_err(), "{what} parsed: {s:?}");
    }
    // A tag this build does not know is named, not guessed at.
    assert_eq!(
        BlockId::from_str(&format!("sausage:{hex64}")),
        Err(IdError::UnknownTag("sausage".into()))
    );
}

/// The tag vocabulary is the substrate's kinds, not a second list beside them.
#[test]
fn the_tags_are_the_kinds() {
    assert_eq!(tag_of(kind::RAW), Some("raw"));
    assert_eq!(tag_of(kind::TREE_NODE), Some("node"));
    assert_eq!(tag_of(200), None);

    // A kind this build has no name for still prints readably and still fails
    // to parse back — it is not silently read as a kind it is not.
    let odd = BlockId::from_parts(200, [7u8; 32]);
    assert!(odd.to_string().starts_with("k200:"), "{odd}");
    assert!(BlockId::from_str(&odd.to_string()).is_err());
}

/// Every vector in the frozen file, checked the way an app would read it back.
#[test]
fn the_block_vectors_parse_back_to_themselves() {
    let mut n = 0;
    for line in std::fs::read_to_string("tests/block_vectors.txt")
        .unwrap()
        .lines()
    {
        let want = line.rsplit(' ').next().unwrap();
        let id = BlockId::from_str(want).unwrap();
        assert_eq!(id.to_string(), want);
        assert_eq!(
            id.kind(),
            if want.starts_with("raw:") {
                kind::RAW
            } else {
                kind::TREE_NODE
            }
        );
        n += 1;
    }
    assert!(n >= 8, "only {n} vectors");
}

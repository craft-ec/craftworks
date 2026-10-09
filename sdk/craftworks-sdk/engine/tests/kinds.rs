//! THE ONE LIST OF BLOCK KINDS (core_types::kind::BlockKind) states each kind's byte, because core-types has no
//! dependencies. The bytes are the SUBSTRATE's (freenet-prolly's `kind`) and the Block contract's (PACK): held equal
//! here, so the one list cannot drift from what the tree hashes and the contract stores.
use core_types::kind::BlockKind;

#[test]
fn the_kind_bytes_are_the_substrates_and_the_contracts() {
    assert_eq!(BlockKind::Raw.byte(), freenet_prolly::kind::RAW);
    assert_eq!(BlockKind::TreeNode.byte(), freenet_prolly::kind::TREE_NODE);
    assert_eq!(BlockKind::Parity.byte(), freenet_prolly::kind::PARITY);
    // The Block contract's PACK: mirrored, as its limits are (pack_format's `the_mirrored_limits_are_the_contracts_numbers`).
    assert_eq!(BlockKind::Pack.byte(), 6);
    assert_eq!(engine::pack::PACK_KIND, BlockKind::Pack.byte());
}

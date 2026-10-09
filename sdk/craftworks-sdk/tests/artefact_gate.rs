//! The ARTEFACT gate: what this SDK writes, run through the contract that
//! will judge it.
//!
//! Replaces `tests/prolly_pin.rs`, which compared a freenet-prolly REVISION
//! string between this repository and the contracts'. That was only ever a
//! proxy for the property, and since freenet-contracts#36 the two revisions
//! are deliberately allowed to differ — so the proxy now fails on correct
//! trees and passes on nothing in particular.
//!
//! The property itself has two halves, and they pull in opposite directions:
//!
//! - **Write arm.** Blocks this SDK's prolly produces must be ACCEPTED by the
//!   released `block.wasm`. A writer that emits something the contract
//!   refuses is a writer whose data never lands.
//! - **Corpus arm.** Blocks a released contract accepted must still PARSE
//!   with this SDK's prolly. A parser that tightens is a reader that cannot
//!   read data already on the network.
//!
//! Stated as one rule: **a newer prolly may only LOOSEN its parser, and may
//! never LOOSEN its writer, relative to the released contract.**
//!
//! "Could not find the wasm" and "could not find the corpus" are FAILURES,
//! not skips. The skew this exists for is invisible to every other test here,
//! because both sides of those tests are the same library.

mod support;
use support::contract::{Contract, Verdict};

/// Where the contracts checkout is.
///
/// `CARGO_MANIFEST_DIR/..` is not enough: work happens in a `git worktree`
/// under a temp dir, and from there the sibling repo is not beside the
/// manifest. The worktree's `.git` is a FILE holding `gitdir: <main>/...`,
/// which names the checkout this one belongs to — read rather than asked for
/// with a subprocess, because the main checkout is shared and a `git` call
/// against a repository someone else is merging in fails for reasons that
/// have nothing to do with this gate.
fn contracts_repo() -> Option<std::path::PathBuf> {
    use std::path::{Path, PathBuf};
    if let Ok(p) = std::env::var("CRAFTWORKS_CONTRACTS") {
        return Some(PathBuf::from(p));
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let beside = manifest.join("../freenet-contracts");
    if beside.join("block/Cargo.toml").is_file() {
        return Some(beside);
    }
    let gitdir = std::fs::read_to_string(manifest.join(".git")).ok()?;
    let common = PathBuf::from(gitdir.trim().strip_prefix("gitdir:")?.trim());
    let common = if common.is_absolute() {
        common
    } else {
        manifest.join(common)
    };
    let main = common
        .ancestors()
        .find(|p| {
            p.join("block/Cargo.toml").is_file() || p.file_name().is_some_and(|n| n == ".git")
        })
        .and_then(|p| {
            if p.file_name().is_some_and(|n| n == ".git") {
                p.parent()
            } else {
                Some(p)
            }
        })?;
    let beside = main.join("../freenet-contracts");
    beside.join("block/Cargo.toml").is_file().then_some(beside)
}

fn block_wasm() -> Vec<u8> {
    let repo = contracts_repo().unwrap_or_else(|| {
        panic!(
            "cannot find the freenet-contracts checkout, so NOTHING was \
             validated through the real contract. Set CRAFTWORKS_CONTRACTS to \
             its path. This is a failure and not a skip: the skew this gate \
             exists for is invisible to every other test in this repository, \
             because both sides of those tests are the same library."
        )
    });
    let path = repo.join("build/block.wasm");
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "the contracts checkout is at {} but {} could not be read ({e}). \
             Run its ./build.sh. Not validating is not passing.",
            repo.display(),
            path.display()
        )
    })
}

/// The gate can run the real contract at all, and the contract can tell a
/// good block from a bad one.
///
/// Every assertion below rests on this: a harness that returned `Invalid` for
/// everything — a mis-built buffer, a misread result — would pass a refusal
/// control and fail the acceptance arm in a way that looks like a writer bug.
#[test]
fn the_gate_can_run_the_contract_and_it_separates_good_from_bad() {
    let mut c = Contract::load(&block_wasm()).expect("the released block.wasm must load");
    // A RAW block: kind byte 0, then the body. Params are blake3 of the whole
    // state, which is exactly what the engine calls the block's id.
    let state = {
        let mut s = vec![freenet_prolly::kind::RAW];
        s.extend_from_slice(b"artefact gate");
        s
    };
    let params = blake3::hash(&state).as_bytes().to_vec();
    assert_eq!(
        c.validate(&params, &state),
        Verdict::Valid,
        "the contract refused a well-formed RAW block, so this harness is not \
         speaking its ABI and no other result here means anything"
    );
    // One byte of the params changed: the state no longer hashes to its key.
    let mut wrong = params.clone();
    wrong[0] ^= 0xFF;
    assert_eq!(
        c.validate(&wrong, &state),
        Verdict::Invalid,
        "the contract ACCEPTED a state that does not hash to its params, so \
         it is not checking and neither is this gate"
    );
}

/// A case the write arm offers the contract, and what it is a boundary of.
struct Case {
    what: &'static str,
    state: Vec<u8>,
}

impl Case {
    fn of(what: &'static str, kind: u8, body: Vec<u8>) -> Case {
        let mut state = Vec::with_capacity(1 + body.len());
        state.push(kind);
        state.extend_from_slice(&body);
        Case { what, state }
    }
    /// A block's params ARE its id: `blake3(kind ‖ body)`.
    fn params(&self) -> Vec<u8> {
        blake3::hash(&self.state).as_bytes().to_vec()
    }
}

/// Build the boundary cases with THIS SDK's prolly.
///
/// On every format boundary the two implementations could disagree about:
/// the largest node, a full pack, parity over a group, and each inline/by
/// reference size class. A gate over comfortable middle-sized blocks would
/// pass while the edges rotted, and the edges are where a writer and a
/// parser drift apart.
fn write_arm_cases() -> Vec<Case> {
    use freenet_prolly::build::TreeBuilder;
    use freenet_prolly::store::MemBlocks;
    use freenet_prolly::{kind, node};

    let mut cases = Vec::new();

    // --- RAW values, at each size class boundary ---
    //
    // MAX_INLINE is where a value stops riding in its leaf and becomes a
    // block of its own, so it is the boundary that decides whether a RAW
    // block exists at all.
    for (what, len) in [
        ("raw: one byte", 1usize),
        ("raw: at MAX_INLINE", node::MAX_INLINE),
        ("raw: one over MAX_INLINE", node::MAX_INLINE + 1),
        ("raw: at MAX_VALUE", node::MAX_VALUE),
    ] {
        cases.push(Case::of(what, kind::RAW, vec![0xA5; len]));
    }

    // --- tree nodes, including one as large as the format allows ---
    //
    // Built by driving the real builder, so these are nodes this SDK would
    // actually write rather than nodes hand-assembled to be near a limit.
    let mut sink = MemBlocks::default();
    let mut b = TreeBuilder::new(|c, bytes: &[u8]| {
        sink.insert(c, bytes);
    });
    // Keys wide enough that the tree is several levels deep, so branch nodes
    // and leaf nodes are both represented.
    for i in 0..4000u32 {
        b.push_bytes(format!("k/{i:06}").as_bytes(), &[(i % 251) as u8; 200])
            .expect("push");
    }
    let root = b.finish().expect("finish");
    let mut biggest = 0usize;
    let mut leaves = 0usize;
    let mut branches = 0usize;
    for (cid, bytes) in sink.0.iter() {
        let _ = cid;
        biggest = biggest.max(bytes.len());
        match node::Node::parse(bytes) {
            Ok(n) if n.is_leaf() => leaves += 1,
            Ok(_) => branches += 1,
            Err(_) => {}
        }
    }
    assert!(
        leaves > 0 && branches > 0,
        "the fixture tree has {leaves} leaves and {branches} branches, so it \
         is not deep enough to exercise both node shapes"
    );
    // Every distinct node the builder produced, capped so the arm stays a
    // gate and not a benchmark. The largest is always included.
    let mut nodes: Vec<(usize, Vec<u8>)> = sink.0.values().map(|b| (b.len(), b.to_vec())).collect();
    nodes.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    for (n, bytes) in nodes.iter().take(24) {
        let what: &'static str = if *n == biggest {
            "tree node: the largest this build wrote"
        } else {
            "tree node"
        };
        cases.push(Case::of(what, kind::TREE_NODE, bytes.clone()));
    }
    let _ = root;

    // --- a tree whose leaves REFERENCE their values ---
    //
    // The tree above keeps every value inline, so its leaves list no parity
    // and every group it has is a branch's. Values in each size class, with
    // enough at MAX_VALUE that their group codes the largest parity the
    // format produces: a symbol as long as a MAX_VALUE member plus framing.
    let mut refs = MemBlocks::default();
    let mut b = TreeBuilder::new(|c, bytes: &[u8]| {
        refs.insert(c, bytes);
    });
    let sizes = [2 * 1024, 8 * 1024, 32 * 1024, node::MAX_VALUE];
    for i in 0..96u32 {
        let len = sizes[(i % 4) as usize];
        b.push_bytes(format!("r/{i:04}").as_bytes(), &vec![(i % 251) as u8; len])
            .expect("push");
    }
    b.finish().expect("finish");
    let ref_leaves: Vec<Vec<u8>> = refs
        .0
        .values()
        .filter(|b| node::Node::parse(b).is_ok_and(|n| n.is_leaf()))
        .cloned()
        .collect();
    assert!(!ref_leaves.is_empty(), "the referencing tree wrote no leaf");
    for bytes in ref_leaves.iter().take(4) {
        cases.push(Case::of(
            "tree node: a leaf listing parity over referenced values",
            kind::TREE_NODE,
            bytes.clone(),
        ));
    }

    // --- parity, coded by this SDK's prolly over both trees' real groups ---
    //
    // `blocks_of` is the one function a writer, a keeper and a repairer all
    // use to produce parity bytes, so these are exactly what goes on the wire.
    let mut parity: Vec<Vec<u8>> = Vec::new();
    let (mut from_branches, mut from_leaves) = (0usize, 0usize);
    for store in [&sink, &refs] {
        for bytes in store.0.values() {
            let Ok(n) = node::Node::parse(bytes) else {
                continue;
            };
            let blocks = freenet_prolly::parity::blocks_of(&n, store)
                .expect("every member of the fixture is held, so its parity codes");
            if n.is_leaf() {
                from_leaves += blocks.len();
            } else {
                from_branches += blocks.len();
            }
            parity.extend(blocks.into_iter().map(|(_, p)| p));
        }
    }
    assert!(
        from_branches > 0 && from_leaves > 0,
        "the fixtures coded {from_branches} branch and {from_leaves} leaf parity \
         block(s); both group shapes must be represented"
    );
    parity.sort_by_key(|p| std::cmp::Reverse(p.len()));
    parity.dedup();
    let largest_parity = parity[0].len();
    assert!(
        largest_parity > node::MAX_VALUE,
        "the largest parity coded is {largest_parity} B, so no group held a \
         MAX_VALUE member and the size boundary is not exercised"
    );
    for (i, p) in parity.iter().take(24).enumerate() {
        let what = if i == 0 {
            "parity: the largest this build coded"
        } else {
            "parity"
        };
        cases.push(Case::of(what, kind::PARITY, p.clone()));
    }
    // Parity is stored with trailing zeros trimmed, so a group whose
    // combination cancels stores an EMPTY body. The contract must keep it.
    cases.push(Case::of(
        "parity: empty (a fully cancelled group)",
        kind::PARITY,
        Vec::new(),
    ));

    // --- packs, built by the engine's own pack builder ---
    //
    // One pack of a commit's nodes, and one filled as close to MAX_PACK as the
    // fixture's members allow (the largest the format takes).
    use engine::pack::{self, MAX_PACK, PACK_HEADER, PACK_KIND};
    let nodes_pack: Vec<(u8, Vec<u8>)> = nodes
        .iter()
        .take(16)
        .map(|(_, b)| (kind::TREE_NODE, b.clone()))
        .collect();
    cases.push(Case::of(
        "pack: a commit's nodes",
        PACK_KIND,
        pack::build(&nodes_pack).expect("the nodes pack builds"),
    ));
    let mut pool: Vec<Vec<u8>> = refs.0.values().chain(sink.0.values()).cloned().collect();
    pool.sort_by_key(|b| std::cmp::Reverse(b.len()));
    let (mut full, mut used) = (Vec::new(), PACK_HEADER);
    for bytes in pool {
        if used + pack::member_cost(bytes.len()) <= MAX_PACK {
            used += pack::member_cost(bytes.len());
            full.push((pack::member_kind(&bytes).byte(), bytes));
        }
    }
    let full = pack::build(&full).expect("the full pack builds");
    assert!(
        full.len() + 1024 > MAX_PACK,
        "the fullest pack is {} B, not within 1 KiB of MAX_PACK ({MAX_PACK})",
        full.len()
    );
    cases.push(Case::of(
        "pack: the fullest the fixture fills",
        PACK_KIND,
        full,
    ));
    cases
}

/// Everything this SDK's prolly writes is accepted by the CURRENT contract.
///
/// The current one, and deliberately not every epoch.
///
/// READABLE under every epoch, WRITABLE under the current one. New writes go
/// to the current epoch only (§3), so a writer has to satisfy that one; the
/// corpus arm is what keeps older epochs' data readable. Conflating the two
/// would make the epoch table read as "we must stay writable under
/// everything ever released", which is a promise nobody made and which the
/// upgrade procedure exists precisely to avoid having to keep.
///
/// `tests/corpus/epochs.md` records the difference this distinction is about:
/// epoch 21ae7e73 predates the parity rule and refuses any branch that lists
/// parity ids — which is every branch this SDK writes. So this SDK cannot
/// write a tree deeper than a leaf under that epoch, and does not have to.
#[test]
fn every_block_this_sdk_writes_is_accepted_by_the_released_contract() {
    let mut c = Contract::load(&block_wasm()).expect("the released block.wasm must load");
    let cases = write_arm_cases();
    assert!(
        cases.len() >= 12,
        "the write arm built only {} case(s); a green arm over almost nothing \
         reads exactly like a green arm over the format",
        cases.len()
    );

    let mut sizes: Vec<usize> = Vec::new();
    for case in &cases {
        let v = c.validate(&case.params(), &case.state);
        assert_eq!(
            v,
            Verdict::Valid,
            "the released contract REFUSED a {} of {} B that this SDK's \
             prolly wrote. A writer that emits what the contract refuses is a \
             writer whose data never lands.",
            case.what,
            case.state.len()
        );
        sizes.push(case.state.len());
    }

    // Every kind this SDK writes is in the arm. A floor on the TOTAL passed
    // for months over RAW and TREE_NODE alone while the doc above promised
    // parity and packs; a per-kind floor is what notices a kind going missing.
    use freenet_prolly::kind;
    let per_kind = |k: u8| cases.iter().filter(|c| c.state[0] == k).count();
    let counts = [
        ("RAW", per_kind(kind::RAW)),
        ("TREE_NODE", per_kind(kind::TREE_NODE)),
        ("PARITY", per_kind(kind::PARITY)),
        ("PACK", per_kind(engine::pack::PACK_KIND)),
    ];
    for (name, n) in counts {
        assert!(n > 0, "the write arm has no {name} case: {counts:?}");
    }

    // The control, and it must EXECUTE: a node one byte over the format's
    // limit is refused. Without it, "everything was accepted" is also what a
    // contract that accepts everything looks like — and the same harness bug
    // that once read every verdict as Valid would pass the arm above.
    let over = Case::of(
        "a node one byte over MAX_NODE",
        freenet_prolly::kind::TREE_NODE,
        vec![0u8; freenet_prolly::node::MAX_NODE + 1],
    );
    assert_eq!(
        c.validate(&over.params(), &over.state),
        Verdict::Invalid,
        "the contract ACCEPTED a node over MAX_NODE, so it is not enforcing \
         the bound and neither is this arm"
    );

    // The same for the two kinds added here: parity one byte over its bound,
    // and a pack carrying a member the contract would refuse on its own.
    let over_parity = Case::of(
        "parity one byte over MAX_PARITY",
        kind::PARITY,
        vec![7u8; engine::pack::MAX_PARITY + 1],
    );
    assert_eq!(
        c.validate(&over_parity.params(), &over_parity.state),
        Verdict::Invalid,
        "the contract ACCEPTED parity over MAX_PARITY"
    );
    let bad_pack = Case::of(
        "a pack with a malformed TREE_NODE member",
        engine::pack::PACK_KIND,
        engine::pack::build(&[
            (kind::RAW, b"ok".to_vec()),
            (kind::TREE_NODE, vec![0xFF; 64]),
        ])
        .expect("builds: the builder does not validate members, the contract does"),
    );
    assert_eq!(
        c.validate(&bad_pack.params(), &bad_pack.state),
        Verdict::Invalid,
        "the contract ACCEPTED a pack whose member it would refuse on its own"
    );

    let biggest = sizes.iter().copied().max().unwrap_or(0);
    println!(
        "  write arm: {} case(s) accepted {counts:?}, largest {biggest} B; \
         controls (node over MAX_NODE, parity over MAX_PARITY, pack with a \
         malformed member) refused",
        cases.len()
    );
}

// ---------------------------------------------------------------------------
// The corpus: what released contracts ACCEPTED, frozen.
// ---------------------------------------------------------------------------

use support::corpus::{self, Entry};

/// Where the frozen corpus lives, in THIS repository.
///
/// In-tree on purpose. The epochs that vouched for it are needed to MAKE it
/// and never to check it, so the gate depends on a file it owns rather than
/// on another repository's build directory being present and current.
fn corpus_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/accepted.bin")
}

/// The epoch wasms, for REGENERATION only.
fn epochs_dir() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("CRAFTWORKS_EPOCHS") {
        return Some(std::path::PathBuf::from(p));
    }
    let repo = contracts_repo()?;
    let beside = repo.join("../freenet-harness/build/epochs");
    beside.join("block-A.wasm").is_file().then_some(beside)
}

/// Build candidates, offer them to EVERY epoch, keep what all of them took.
///
/// Run by hand, never as a gate:
///
/// ```text
/// cargo test -p craftworks-sdk --test artefact_gate -- --ignored regenerate
/// ```
///
/// It is `#[ignore]` because it needs artefacts from another repository and
/// WRITES to this one. The gate it feeds needs neither.
#[test]
#[ignore = "regenerates the frozen corpus; needs the epoch wasms"]
fn regenerate_the_corpus() {
    let dir = epochs_dir().expect(
        "cannot find the epoch wasms. Set CRAFTWORKS_EPOCHS to the directory \
         holding block-A.wasm, block-B.wasm, register-A.wasm, register-B.wasm.",
    );
    let load = |name: &str| -> (String, Contract) {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| {
            panic!("reading {name}: {e}");
        });
        let short = blake3::hash(&bytes); // only for the message below
        let _ = short;
        let sha = sha256_short(&bytes);
        (
            sha,
            Contract::load(&bytes).unwrap_or_else(|e| panic!("{name}: {e}")),
        )
    };
    let mut block_epochs = [load("block-A.wasm"), load("block-B.wasm")];
    let mut register_epochs = [load("register-A.wasm"), load("register-B.wasm")];
    println!(
        "  block epochs:    {}",
        block_epochs
            .iter()
            .map(|(h, _)| h.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  register epochs: {}",
        register_epochs
            .iter()
            .map(|(h, _)| h.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut kept: Vec<Entry> = Vec::new();
    let mut offered = 0usize;
    let mut partial = 0usize;
    // (shape, body length, the epochs that took it) — written out beside the
    // corpus, because WHICH epochs disagree and about WHAT is a fact about
    // the released contracts and belongs in the repository, not in a console
    // line nobody sees again.
    let mut provenance: Vec<(String, usize, Vec<String>)> = Vec::new();

    // --- blocks ---
    for case in write_arm_cases() {
        offered += 1;
        let params = case.params();
        let mut accepted_by = Vec::new();
        for (hash, c) in block_epochs.iter_mut() {
            match c.validate(&params, &case.state) {
                Verdict::Valid => accepted_by.push(hash.clone()),
                Verdict::Invalid => {}
                other => panic!("block epoch {hash} broke on {}: {other:?}", case.what),
            }
        }
        // ONLY what every epoch took. An entry one epoch refused is not
        // evidence about what the network accepts; it is evidence the epochs
        // disagree, which is a different finding and not this corpus's job.
        if case.state[0] == freenet_prolly::kind::TREE_NODE {
            let body = &case.state[1..];
            let shape = match freenet_prolly::node::Node::parse(body) {
                Ok(n) if n.is_leaf() => "leaf",
                Ok(_) => "branch",
                Err(_) => "unparseable",
            };
            provenance.push((shape.to_string(), case.state.len() - 1, accepted_by.clone()));
        }
        if accepted_by.len() == block_epochs.len() {
            kept.push(Entry {
                contract: "block".into(),
                epochs: accepted_by,
                what: case.what.to_string(),
                params,
                state: case.state,
            });
        } else if !accepted_by.is_empty() {
            // The epochs DISAGREE about these bytes. Named, not counted: a
            // case one released contract takes and another refuses is a fact
            // about the epochs, and burying it in a tally is how it stays
            // unnoticed.
            partial += 1;
        }
    }

    // --- register head records ---
    for (what, params, state) in register_cases() {
        offered += 1;
        let mut accepted_by = Vec::new();
        for (hash, c) in register_epochs.iter_mut() {
            match c.validate(&params, &state) {
                Verdict::Valid => accepted_by.push(hash.clone()),
                Verdict::Invalid => {}
                other => panic!("register epoch {hash} broke on {what}: {other:?}"),
            }
        }
        if accepted_by.len() == register_epochs.len() {
            kept.push(Entry {
                contract: "register".into(),
                epochs: accepted_by,
                what: what.to_string(),
                params,
                state,
            });
        } else if !accepted_by.is_empty() {
            partial += 1;
        }
    }

    assert!(
        kept.iter().any(|e| e.contract == "block"),
        "no block was accepted by every epoch, so the corpus would have no \
         block arm at all"
    );
    assert!(
        kept.iter().any(|e| e.contract == "register"),
        "no register record was accepted by every epoch"
    );

    let path = corpus_path();
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, corpus::encode(&kept)).expect("writing the corpus");

    // What each epoch took, by node shape. A disagreement between released
    // contracts is the most interesting thing this generator can find, and
    // burying it in a tally is how it stays unnoticed.
    let mut by_shape: std::collections::BTreeMap<(String, String), usize> = Default::default();
    for (shape, _len, eps) in &provenance {
        let key = (shape.clone(), eps.join("+"));
        *by_shape.entry(key).or_default() += 1;
    }
    let mut note = String::new();
    note.push_str("# Which released epochs accepted what\n\n");
    note.push_str("Written by `regenerate_the_corpus`. Only entries EVERY epoch\n");
    note.push_str("accepted are frozen in accepted.bin; this records the rest,\n");
    note.push_str("because a case one released contract takes and another refuses\n");
    note.push_str("is a fact about the epochs and not a rounding error.\n\n");
    for ((shape, eps), n) in &by_shape {
        note.push_str(&format!("- {n:3} x {shape:<8} accepted by: {eps}\n"));
    }
    std::fs::write(path.parent().expect("a parent").join("epochs.md"), note)
        .expect("writing the provenance note");
    println!(
        "  wrote {} entries of {offered} offered ({partial} accepted by some \
         epochs but not all, and therefore NOT frozen) to {}",
        kept.len(),
        path.display()
    );
}

fn sha256_short(bytes: &[u8]) -> String {
    // The epochs are named by sha256 in hashes.toml, so the corpus names them
    // the same way — a corpus that identified an epoch differently from the
    // table everyone else reads would be one nobody could cross-check.
    use std::process::Command;
    let tmp = std::env::temp_dir().join(format!("epoch-{}.wasm", std::process::id()));
    std::fs::write(&tmp, bytes).expect("temp");
    let out = Command::new("shasum")
        .args(["-a", "256", tmp.to_str().expect("path")])
        .output()
        .expect("shasum");
    let _ = std::fs::remove_file(&tmp);
    String::from_utf8_lossy(&out.stdout)
        .chars()
        .take(16)
        .collect()
}

/// Head records for the Register corpus, signed here.
///
/// The Register's mode 0 is a single writer, which is what a device's own
/// head is. Each case is a `(what, params, state)` on a boundary: the
/// smallest value, a root-sized one, and the largest a record may carry.
fn register_cases() -> Vec<(&'static str, Vec<u8>, Vec<u8>)> {
    use ed25519_dalek::{Signer, SigningKey};

    // A fixed key, so regenerating the corpus twice gives the same bytes and
    // a diff on it means something changed rather than that it was re-run.
    let sk = SigningKey::from_bytes(&[11u8; 32]);
    let vk = sk.verifying_key();
    let mut params = Vec::from(*b"RG01");
    params.push(0u8);
    params.extend_from_slice(&vk.to_bytes());
    params.extend_from_slice(b"head");
    let params_hash: [u8; 32] = *blake3::hash(&params).as_bytes();

    let mut out = Vec::new();
    for (what, seq, value) in [
        ("register: a one-byte value", 1u64, [0x01u8; 1].to_vec()),
        ("register: a root-sized value", 2, [0x5Au8; 32].to_vec()),
        ("register: at MAX_VALUE", 3, vec![0x77u8; 4096]),
    ] {
        let value_hash: [u8; 32] = *blake3::hash(&value).as_bytes();
        let mut msg = Vec::from(*b"RG01-sig");
        msg.extend_from_slice(&params_hash);
        msg.push(0u8); // not terminal
        msg.extend_from_slice(&seq.to_le_bytes());
        msg.extend_from_slice(&value_hash);
        let sig = sk.sign(&msg).to_bytes();

        let mut state = Vec::from(*b"RG01");
        state.push(0b01); // a record, no equivocation evidence
        state.push(0u8); // not terminal
        state.extend_from_slice(&seq.to_le_bytes());
        state.extend_from_slice(&(value.len() as u16).to_le_bytes());
        state.extend_from_slice(&value);
        state.extend_from_slice(&sig);
        out.push((what, params.clone(), state));
    }
    out
}

/// Everything a released contract accepted still PARSES with this prolly.
///
/// The other half of the rule, and it pulls the other way: the write arm says
/// a newer writer may not emit what the contract refuses; this says a newer
/// PARSER may not refuse what a contract already accepted. Data on the
/// network was written under an older epoch and a reader that tightened
/// cannot read it — a failure that shows up as missing data, long after the
/// change that caused it.
///
/// Stated once, for whoever changes prolly next: **a newer prolly may only
/// LOOSEN its parser, and may never LOOSEN its writer, relative to the
/// released contract.**
#[test]
fn every_block_a_released_epoch_accepted_still_parses() {
    let path = corpus_path();
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "the frozen corpus at {} could not be read ({e}). It is committed \
             to this repository, so this is not a missing dependency — it is a \
             gate with nothing to check, which is not a pass. Regenerate with \
             `cargo test --test artefact_gate -- --ignored regenerate`.",
            path.display()
        )
    });
    let entries = corpus::decode(&bytes).expect("the corpus must decode");

    let mut blocks = 0usize;
    let mut registers = 0usize;
    for e in &entries {
        assert!(
            !e.epochs.is_empty(),
            "corpus entry {:?} names no epoch that accepted it, so it is not \
             evidence of anything",
            e.what
        );
        match e.contract.as_str() {
            "block" => {
                // The params ARE the block's id, so this checks the identity
                // as well as the shape: a parser that read the bytes but
                // disagreed about what they hash to would still be one that
                // cannot find them.
                assert_eq!(
                    blake3::hash(&e.state).as_bytes().as_slice(),
                    e.params.as_slice(),
                    "corpus entry {:?}: its state no longer hashes to its \
                     params, so the corpus file is damaged",
                    e.what
                );
                let kind = e.state[0];
                let body = &e.state[1..];
                if kind == freenet_prolly::kind::TREE_NODE {
                    freenet_prolly::node::Node::parse(body).unwrap_or_else(|err| {
                        panic!(
                            "THIS PROLLY CANNOT PARSE a node that epochs {:?} \
                             accepted ({:?}, {} B): {err:?}. A parser that \
                             tightened cannot read data already on the \
                             network.",
                            e.epochs,
                            e.what,
                            body.len()
                        )
                    });
                }
                blocks += 1;
            }
            "register" => {
                let (seq, _value) = parse_head(&e.state).unwrap_or_else(|| {
                    panic!(
                        "this build cannot read a head record that epochs {:?} \
                         accepted ({:?})",
                        e.epochs, e.what
                    )
                });
                assert!(seq > 0, "a head at seq 0 is not one of ours");
                registers += 1;
            }
            other => panic!("corpus entry names an unknown contract {other:?}"),
        }
    }

    assert!(
        blocks >= 8,
        "the corpus holds only {blocks} block(s); a green arm over almost \
         nothing reads exactly like a green arm over the format"
    );
    assert!(
        registers >= 3,
        "the corpus holds only {registers} register record(s)"
    );

    // The control, and it must EXECUTE: a corpus entry with one byte flipped
    // must FAIL. Without it, "everything parsed" is also what a parser that
    // accepts anything looks like — and a corpus of bytes nobody checks is a
    // file, not a gate.
    let mut refused = 0usize;
    let mut tried = 0usize;
    for e in entries.iter().filter(|e| e.contract == "block") {
        let kind = e.state[0];
        if kind != freenet_prolly::kind::TREE_NODE {
            continue;
        }
        tried += 1;
        // A byte inside the node's header, where the format has structure to
        // disagree with. Flipping a byte in a value's payload would often be
        // legal, and a control that passes for that reason is not one.
        let mut damaged = e.state[1..].to_vec();
        damaged[1] ^= 0xFF;
        if freenet_prolly::node::Node::parse(&damaged).is_err() {
            refused += 1;
        }
    }
    assert!(
        tried > 0,
        "no node in the corpus to damage, so no control ran"
    );
    assert_eq!(
        refused, tried,
        "only {refused} of {tried} damaged nodes were refused, so the parser \
         accepts bytes it should not and the arm above proves nothing"
    );

    println!(
        "  corpus arm: {blocks} block(s) and {registers} head record(s) \
         accepted by released epochs still parse; {refused}/{tried} damaged \
         nodes refused"
    );
}

/// Read `(seq, value)` out of an encoded Register state.
fn parse_head(state: &[u8]) -> Option<(u64, Vec<u8>)> {
    let rest = state.strip_prefix(b"RG01")?;
    let (&flags, rest) = rest.split_first()?;
    if flags & 0b01 == 0 {
        return None;
    }
    let (_terminal, rest) = rest.split_first()?;
    let (seq, rest) = rest.split_at_checked(8)?;
    let seq = u64::from_le_bytes(seq.try_into().ok()?);
    let (vlen, rest) = rest.split_at_checked(2)?;
    let vlen = u16::from_le_bytes([vlen[0], vlen[1]]) as usize;
    Some((seq, rest.get(..vlen)?.to_vec()))
}

/// Writable under the CURRENT epoch; readable under every epoch.
///
/// Two different obligations, and the corpus table invites conflating them.
/// This states the split in an assertion so it cannot be read the other way:
/// the corpus (what must stay READABLE) is allowed to contain entries whose
/// epoch lists differ, while the write arm targets one contract — the one in
/// the contracts build, which is the current epoch.
#[test]
fn the_write_arm_targets_the_current_epoch_only() {
    // The contract the write arm validates against IS the current build.
    let current = block_wasm();
    let repo = contracts_repo().expect("the contracts checkout");
    let named = std::fs::read_to_string(repo.join("build/hashes.toml"))
        .expect("build/hashes.toml, written by the contracts build");
    // hashes.toml records what that build produced. Reading it here is not a
    // gate on the hash — `released.toml`'s own note is emphatic that nothing
    // should gate on a hash table — it is a check that the wasm the write arm
    // loaded is the one that build wrote, rather than a stale file.
    let want = named
        .lines()
        .find_map(|l| l.trim().strip_prefix("block = \"sha256:"))
        .map(|v| v.trim_end_matches('"').to_string())
        .expect("hashes.toml names a block hash");
    let got = sha256_short(&current);
    assert!(
        want.starts_with(&got),
        "the write arm validated against a block.wasm ({got}...) that is not \
         the one the contracts build produced ({}...). A stale artefact reads \
         exactly like a passing gate.",
        &want[..16.min(want.len())]
    );

    // And the corpus is the OTHER obligation. It may hold entries accepted by
    // epochs the write arm does not target — that is the point of it.
    let entries =
        corpus::decode(&std::fs::read(corpus_path()).expect("the corpus")).expect("decodes");
    let epoch_names: std::collections::BTreeSet<&str> = entries
        .iter()
        .flat_map(|e| e.epochs.iter().map(|s| s.as_str()))
        .collect();
    assert!(
        epoch_names.len() >= 2,
        "the corpus names {} epoch(s); with only one there is nothing to be \
         readable-across and the distinction this test draws is empty",
        epoch_names.len()
    );
    println!(
        "  writable under the current epoch ({got}...); readable across {} \
         epochs in the corpus",
        epoch_names.len()
    );
}

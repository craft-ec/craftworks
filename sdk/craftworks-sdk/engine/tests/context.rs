//! The engine's own bounds: the empty leaf, the context BUDGET its
//! bookkeeping is still held to (`context_len`, `keep_saveable`; whether that
//! budget still earns its verdicts is sdk#385), the commit block cap, no
//! global state, and what a fresh engine (a page reload) answers.
//!
//! The context CARRY (`to_context` / `from_context`, a delegate rebuilt from
//! its bytes each call) was deleted in #305: the engine lives in the page.

mod common;
use engine::{ClientId, Effect, Engine, Event, Op, Params, WriteId};
use freenet_prolly::store::{Blocks, MemBlocks};
use freenet_prolly::Cid;

/// A block source a test can fill, standing in for the node's own store.
#[derive(Default, Clone)]
struct Store(MemBlocks);

impl Blocks for Store {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.0.get(cid)
    }
}

impl Store {
    fn put(&mut self, id: Cid, bytes: &[u8]) {
        self.0.insert(id, bytes);
    }
    /// Absorb what a commit emitted, the way a node does once the puts land.
    fn absorb(&mut self, effects: &[Effect]) {
        for f in effects {
            match f {
                Effect::PutPack { id, bytes, .. } => {
                    for (mid, mbytes) in engine::pack::members(bytes) {
                        self.put(mid, &mbytes);
                    }
                    self.put(*id, bytes);
                }
                Effect::PutBlock { id, bytes, .. } => {
                    self.put(*id, bytes)
                }
                other => common::no_answer_owed(other),
            }
        }
    }
}

/// The empty leaf is the ONLY block the core can produce without its source,
/// and its bytes must hash to the id the format defines.
///
/// It is a pure function of the format, not a cache — but a constant that
/// drifts from the format is worse than no constant at all: it would name a
/// root nothing can produce, on a device that has never written. So the id is
/// checked against the library's own, every build.
#[test]
fn the_empty_leaf_is_the_only_block_the_core_knows_and_it_hashes_to_its_id() {
    let leaf = freenet_prolly::chunk::empty_leaf();
    assert_eq!(
        leaf.cid,
        freenet_prolly::block_id(freenet_prolly::kind::TREE_NODE, &leaf.bytes),
        "the empty leaf's bytes do not hash to its id: the constant has drifted \
         from the format"
    );

    // An engine with an EMPTY source can still read its own empty tree, and
    // gets "no such key" rather than "unavailable".
    //
    // "A device with no head" is a FACT the engine learns, not its starting
    // assumption: a read before the head is recovered waits for it (sdk#223).
    // So the head is read, and found missing, first — and that costs no block.
    let mut e = Engine::new(Params::default(), Store::default());
    let _ = e.step(Event::Start {
        key: engine::KeySource::SecretStore,
        epochs: vec![engine::Epoch(1)],
    });
    let _ = e.step(Event::HeadMissing);
    let out = e.step(Event::Get {
        client: ClientId(1),
        req_id: engine::read::ReqId(1),
        key: b"anything".to_vec(),
    });
    let replied = out.iter().any(|f| {
        matches!(
            f,
            Effect::Reply {
                result: engine::read::ReadResult::Value(None),
                ..
            }
        )
    });
    assert!(
        replied,
        "a device with no head could not read its own empty tree: {out:?}"
    );
    // And it asked the network for nothing to do it.
    assert!(
        !out.iter().any(|f| matches!(f, Effect::FetchBlock { .. })),
        "reading an empty tree cost a fetch"
    );
}

/// What the context actually costs, for the shapes that can grow.
///
/// Numbers, not adjectives: the platform gives a delegate 400 KiB of context
/// (`DelegateContext::MAX_SIZE` = 4096*10*10), and every cap in `Params` is a
/// slice of that. This prints the measured size of each shape so the caps can
/// be read against something, and asserts the ones that must stay small.
#[test]
fn the_context_costs_what_it_is_budgeted() {
    const PLATFORM: usize = 4096 * 10 * 10;
    let p = Params::default();

    // Idle: a started engine with a head and nothing in flight.
    let mut store = Store::default();
    let mut e = Engine::new(p, store.clone());
    let _ = e.step(Event::Start {
        key: engine::KeySource::SecretStore,
        epochs: vec![engine::Epoch(1)],
    });
    let idle = e.context_len();

    // One commit in flight over ~1 MiB of values. This is the shape that
    // grows with the SIZE of a write: the commit's bookkeeping names every
    // block it is waiting on, one Cid each.
    //
    // The values are over `max_packed_value`, so each is PUT on its own. A
    // first attempt used 4 KiB values, and the whole megabyte rode in ONE
    // pack -- one id in the bookkeeping, and a number that said nothing about
    // per-block cost. The `blocks > 10` floor below is what caught it.
    let mut e = Engine::new(p, store.clone());
    let ops: Vec<(Vec<u8>, Op)> = (0..16u32)
        .map(|i| {
            (
                format!("k/{i:05}").into_bytes(),
                Op::Put(vec![(i % 251) as u8; p.max_packed_value + 1]),
            )
        })
        .collect();
    // A new tree: nothing to recover. A write before recovery waits (sdk#223).
    let _ = e.step(Event::HeadMissing);
    let out = e.step(Event::forced_write(ClientId(1), WriteId(1), ops));
    store.absorb(&out);
    let in_flight = e.context_len();
    let blocks = out
        .iter()
        .filter(|f| {
            matches!(
                f,
                Effect::PutPack { .. } | Effect::PutBlock { .. }
            )
        })
        .count();

    println!("  idle:                      {idle:6} B");
    println!("  ~1 MiB commit in flight:   {in_flight:6} B  ({blocks} blocks)");
    println!(
        "  ...per block:              {:6} B",
        (in_flight - idle) / blocks.max(1)
    );
    println!("  platform limit:            {PLATFORM:6} B");
    println!("  max_context_bytes:         {:6} B", p.max_context_bytes);

    assert!(
        idle < 1024,
        "an idle engine carries {idle} B of context; it holds a head and \
         nothing else"
    );
    assert!(
        p.max_context_bytes < PLATFORM,
        "max_context_bytes ({}) leaves no headroom under the platform's {PLATFORM} B",
        p.max_context_bytes
    );
    // The point of printing `blocks`: if a 1 MiB write produced two blocks,
    // the in-flight number would be small for a reason that says nothing
    // about the shape this is measuring.
    assert!(
        blocks > 10,
        "a 1 MiB write produced only {blocks} block(s), so this does not \
         measure a commit's per-block bookkeeping"
    );
    assert!(
        in_flight < p.max_context_bytes,
        "a 1 MiB commit's context is {in_flight} B, over the {} B budget",
        p.max_context_bytes
    );
}
/// Every shape at its cap still fits the budget, and a burst past a cap is
/// answered rather than allowed to destroy the context.
///
/// The measured costs: an idle engine is 125 B, a parked read about 131 B,
/// and a commit's bookkeeping about 60 B per block in flight. Unbounded, ten
/// thousand parked reads make a 1.31 MB context — over three times the
/// platform's 400 KiB — and in the delegate era saving it failed, taking the
/// IN-FLIGHT COMMIT with it. The budget still holds (sdk#385 asks whether it
/// should).
///
/// The shapes are measured APART and summed, because they cannot be built
/// together: a commit needs a tree path it can read, and a parked read is one
/// that could not read the tree. Summing is the conservative direction. A
/// parked write and a pending commit are genuinely exclusive — a write is
/// refused while either is set — so the worst case is reads plus the LARGER
/// of those two, and the arithmetic below says so rather than assuming it.
#[test]
fn the_budget_holds_with_every_shape_at_its_cap() {
    let p = Params::default();
    let store = Store::default();
    let idle = {
        let mut e = Engine::new(p, store.clone());
        let _ = e.step(Event::Start {
            key: engine::KeySource::SecretStore,
            epochs: vec![engine::Epoch(1)],
        });
        e.context_len()
    };

    // --- reads at their cap, and a burst well past it ---
    let mut e = Engine::new(p, store.clone());
    let _ = e.step(Event::Start {
        key: engine::KeySource::SecretStore,
        epochs: vec![engine::Epoch(1)],
    });
    // A root the store does not hold, so every read parks rather than being
    // answered from the empty tree.
    let _ = e.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root: [7u8; 32],
    });
    let burst = p.max_parked_reads * 10;
    let mut refused = 0usize;
    for i in 0..burst {
        let out = e.step(Event::Get {
            client: ClientId(1),
            req_id: engine::read::ReqId(i as u64),
            key: format!("k/{i:09}").into_bytes(),
        });
        if out.iter().any(|f| {
            matches!(
                f,
                Effect::Reply {
                    result: engine::read::ReadResult::Unavailable(_),
                    ..
                }
            )
        }) {
            refused += 1;
        }
    }
    let reads = e.context_len();
    assert_eq!(
        refused,
        burst - p.max_parked_reads,
        "{refused} of {burst} reads were refused; every read past the cap \
         owes the caller an answer, and none before it may be refused"
    );

    // --- a commit at the block cap, measured on a tree that can be read ---
    // Against the empty tree, so the apply succeeds and a commit really does
    // open. An earlier version measured this on the fake root above: the
    // apply stopped on a cold block, no commit opened, and "a commit fits
    // beside the reads" was asserted over a context with no commit in it.
    let mut e2 = Engine::new(p, store.clone());
    let _ = e2.step(Event::Start {
        key: engine::KeySource::SecretStore,
        epochs: vec![engine::Epoch(1)],
    });
    // Values over max_packed_value are PUT one each, so the block count is
    // the number of keys and the cap is reachable without 8 MiB of data.
    let n = 64u32;
    let ops: Vec<(Vec<u8>, Op)> = (0..n)
        .map(|i| {
            (
                format!("w/{i:05}").into_bytes(),
                Op::Put(vec![(i % 251) as u8; p.max_packed_value + 1]),
            )
        })
        .collect();
    // A new tree: nothing to recover. A write before recovery waits (sdk#223).
    let _ = e2.step(Event::HeadMissing);
    let out = e2.step(Event::forced_write(ClientId(1), WriteId(1), ops));
    let commit_blocks = out
        .iter()
        .filter(|f| matches!(f, Effect::PutPack { .. } | Effect::PutBlock { .. }))
        .count();
    assert!(
        commit_blocks >= n as usize,
        "the commit named {commit_blocks} block(s) for {n} oversized values, \
         so this does not measure per-block bookkeeping"
    );
    let commit = e2.context_len();
    let per_block = (commit - idle) / commit_blocks;
    // What the cap costs, at the per-block rate this just measured.
    let commit_worst = idle + per_block * p.max_commit_blocks;
    // The parked write is fetch state only (R-b: its ops are in the page's
    // queue): the path it waits on.
    let write_worst = idle + 64 * 32 + 128;
    let worst = reads + commit_worst.max(write_worst);

    println!("  idle:                        {idle:6} B");
    println!(
        "  {:4} parked reads (the cap):  {reads:6} B",
        p.max_parked_reads
    );
    println!("  commit of {commit_blocks:3} blocks:        {commit:6} B  ({per_block} B/block)");
    println!(
        "  commit at its cap ({:4}):    {commit_worst:6} B",
        p.max_commit_blocks
    );
    println!("  parked write at its cap:     {write_worst:6} B");
    println!("  WORST CASE:                  {worst:6} B");
    println!("  budget:                      {:6} B", p.max_context_bytes);
    println!("  platform:                    {:6} B", 4096 * 10 * 10);

    assert!(
        worst < p.max_context_bytes,
        "every shape at its cap costs {worst} B, over the {} B budget: the \
         caps do not add up and a legal state cannot be written",
        p.max_context_bytes
    );
    // ...and the test is near the thing it claims to check. Without this,
    // caps of 1 would pass it.
    assert!(
        worst > p.max_context_bytes / 2,
        "the worst case is {worst} B against a {} B budget, so this asserts \
         almost nothing about the caps",
        p.max_context_bytes
    );
}

/// The commit block cap refuses the write, and leaves no trace.
#[test]
fn a_commit_over_the_block_cap_is_refused_and_a_smaller_one_is_not() {
    let cap = 8usize;
    let p = Params {
        max_commit_blocks: cap,
        ..Params::default()
    };
    let store = Store::default();
    let big = |n: u32| -> Vec<(Vec<u8>, Op)> {
        (0..n)
            .map(|i| {
                (
                    format!("w/{i:05}").into_bytes(),
                    Op::Put(vec![(i % 251) as u8; p.max_packed_value + 1]),
                )
            })
            .collect()
    };
    let run = |ops: Vec<(Vec<u8>, Op)>| -> (Vec<Effect>, Cid, Cid) {
        let mut e = Engine::new(p, store.clone());
        let _ = e.step(Event::Start {
            key: engine::KeySource::SecretStore,
            epochs: vec![engine::Epoch(1)],
        });
        let before = e.root();
        // A new tree: nothing to recover. A write before recovery waits (sdk#223).
        let _ = e.step(Event::HeadMissing);
        let out = e.step(Event::forced_write(ClientId(1), WriteId(1), ops));
        let after = e.root();
        (out, before, after)
    };

    let (out, before, after) = run(big(cap as u32 * 4));
    // Refused as NEVER acceptable, with the count -- not `Busy`, which the
    // outbox re-sends for ever (craftworks-sdk#136).
    assert!(
        out.iter().any(|f| matches!(
            f,
            Effect::Notify {
                state: engine::State::TooLarge { bound: engine::WriteBound::CommitBlocks, limit, got },
                ..
            } if *limit == cap && *got > cap
        )),
        "a write naming more than {cap} blocks was not refused TooLarge"
    );
    assert_eq!(
        before, after,
        "the refused write changed the tree, so it was applied AND refused"
    );

    // The control: a write UNDER the cap, otherwise identical, is accepted.
    let (out, before, after) = run(big(2));
    assert!(
        out.iter().any(|f| matches!(
            f,
            Effect::Notify {
                state: engine::State::Accepted,
                ..
            }
        )),
        "a write under the same cap was refused too, so the cap is not what \
         refused the larger one"
    );
    assert_ne!(before, after, "the accepted write did not reach the tree");
    println!("  over the block cap: Busy, tree untouched; under it: Accepted");
}

/// The engine keeps nothing outside its context.
///
/// A delegate gets a fresh wasm instance, and a fresh linear memory, on every
/// `inbound_app_message`. A `static mut`, a `thread_local!`, a `lazy_static`
/// or a `OnceLock` in the engine would work perfectly in every test here — one
/// process, one memory — and silently hold nothing at all in production. No
/// test can observe that, because the test harness IS the single process the
/// defect needs; so this reads the source.
///
/// The source is EMBEDDED, not read from disk. Reading it needed
/// `CARGO_MANIFEST_DIR`, which `env!` bakes in at COMPILE time — and this
/// workspace shares one `CARGO_TARGET_DIR` across git worktrees, so a cached
/// test binary can carry a path belonging to a checkout that no longer
/// exists. This gate then failed with `NotFound` on a tree whose engine was
/// perfectly clean, which for a gate is the worst outcome: one that cries
/// wolf is one people learn to re-run and then to ignore. `include_str!`
/// resolves relative to THIS file at compile time and embeds the bytes, so
/// the gate reads the exact source its own binary was built from and touches
/// no filesystem at all.
///
/// Completeness is the other half, and it is checked against the directory
/// when the directory happens to be readable: a module added and not listed
/// here would otherwise go unscanned for ever.
#[test]
fn no_global_state_in_the_engine() {
    const SOURCES: [(&str, &str); 9] = [
        ("src/lib.rs", include_str!("../src/lib.rs")),
        ("src/race_get.rs", include_str!("../src/race_get.rs")),
        ("src/asks.rs", include_str!("../src/asks.rs")),
        ("src/read.rs", include_str!("../src/read.rs")),
        ("src/pack.rs", include_str!("../src/pack.rs")),
        ("src/subs.rs", include_str!("../src/subs.rs")),
        ("src/repair.rs", include_str!("../src/repair.rs")),
        ("src/wanted.rs", include_str!("../src/wanted.rs")),
        ("src/commit.rs", include_str!("../src/commit.rs")),
    ];
    let pattern = [
        "static ",
        "thread_local!",
        "lazy_static!",
        "OnceLock",
        "OnceCell",
    ];

    let mut lines = 0usize;
    let mut found: Vec<String> = Vec::new();
    for (name, text) in SOURCES {
        for (n, line) in text.lines().enumerate() {
            lines += 1;
            let code = line.trim_start();
            // `const` is fine: it is inlined, not stored.
            if code.starts_with("//") || code.starts_with("pub const") || code.starts_with("const")
            {
                continue;
            }
            if pattern.iter().any(|p| code.contains(p)) {
                found.push(format!("{name}:{}: {code}", n + 1));
            }
        }
    }

    assert!(
        lines > 100,
        "the scan read {lines} line(s) of engine source, so it checked nothing"
    );
    assert!(
        found.is_empty(),
        "the engine holds global state, which every engine in the process \
         would share:\n{}",
        found.join("\n")
    );

    // The pattern really matches: this file's own store has a deliberate,
    // test-only `thread_local!`, and the same scan finds it.
    let support = include_str!("common/mod.rs");
    let hits = support
        .lines()
        .filter(|l| pattern.iter().any(|p| l.trim_start().contains(p)))
        .count();
    assert!(
        hits > 0,
        "the scan found no global in tests/common/mod.rs either, which HAS \
         one on purpose — so the pattern matches nothing and the clean result \
         above means nothing"
    );

    // Completeness, when the directory can be read. Not a skip that passes:
    // if the directory IS there and holds a module this test does not list,
    // that module is unscanned and this fails.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            let on_disk: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".rs"))
                .collect();
            let listed: Vec<&str> = SOURCES
                .iter()
                .map(|(n, _)| n.trim_start_matches("src/"))
                .collect();
            for f in &on_disk {
                assert!(
                    listed.contains(&f.as_str()),
                    "engine/src/{f} exists and this gate does not scan it: add \
                     it to SOURCES"
                );
            }
            println!(
                "  {} embedded file(s), {lines} lines, no globals (pattern \
                 verified on {hits} test-only hit(s); {} file(s) on disk, all \
                 listed)",
                SOURCES.len(),
                on_disk.len()
            );
        }
        Err(e) => println!(
            "  {} embedded file(s), {lines} lines, no globals (pattern \
             verified on {hits} test-only hit(s); the src directory was not \
             readable -- {e} -- so completeness was not re-checked, but every \
             listed file WAS scanned)",
            SOURCES.len()
        ),
    }
}


/// A page RELOAD costs a restart, not correctness.
///
/// A fresh engine starts from `Start`, re-reads its head, and gives a client
/// asking about the write the old engine had in flight NO verdict: a fresh
/// engine cannot tell a write that died from one whose head landed before the
/// loss, so `Lost` would be a guess (WRITE-PATH.md session table; sdk#196).
/// The client's own timeout hands the write back.
#[test]
fn a_fresh_engine_recovers_from_the_head_and_gives_the_unknown_write_no_verdict() {
    let mut store = Store::default();
    let mut e: Engine<Store> = Engine::new(Params::default(), store.clone());
    // A new tree: nothing to recover. A write before recovery waits (sdk#223).
    let _ = e.step(Event::HeadMissing);
    let out = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), Op::Put(vec![5u8; 40]))]));
    store.absorb(&out);
    let published = e.published_root();

    // The page reloads: a new engine over the node's blocks.
    let mut e2: Engine<Store> = Engine::new(Params::default(), store.clone());
    let out = e2.step(Event::Start {
        key: engine::KeySource::SecretStore,
        epochs: vec![engine::Epoch(1)],
    });
    assert!(
        out.iter().any(|f| matches!(f, Effect::ReadHead { .. })),
        "a fresh engine did not re-read its head, so a reload loses the tree \
         as well as the bookkeeping"
    );
    let _ = e2.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root: published,
    });
    assert_eq!(
        e2.published_root(),
        published,
        "the recovered engine did not take the head it was given"
    );

    let out = e2.step(Event::AskWrite {
        client: ClientId(1),
        write_id: WriteId(1),
    });
    assert_eq!(
        out.iter()
            .filter_map(|f| match f {
                Effect::Notify { state, .. } => Some(*state),
                _ => None,
            })
            .collect::<Vec<_>>(),
        Vec::<engine::State>::new(),
        "a client asking about a write this fresh engine has no record of was \
         told something. Not knowing is not evidence (sdk#196 review): the \
         write may be one that published and whose Published was lost, so \
         `Lost` would roll back a write that is in the tree. No verdict; the \
         client's own timeout decides"
    );
    println!(
        "  reload: fresh start, head re-read, the unknown write answered with no verdict"
    );
}

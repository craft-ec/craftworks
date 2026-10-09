//! MEASUREMENT for READ-STATE.md (design B's costs), not a gate test.
//! Native release build of the in-page engine against the testkit node.
use protocol::{Bound, Op, Reply, Request};
use std::time::Instant;
use testkit::page_node::{PageConn, PageNode, Served};

fn replies(frames: &[Vec<u8>]) -> Vec<Reply> {
    frames
        .iter()
        .filter_map(|f| protocol::decode_reply(f).ok())
        .collect()
}

fn scan_all(c: &mut PageConn, req: &mut u64) -> (usize, [u8; 32]) {
    let (mut n, mut after) = (0usize, None);
    loop {
        *req += 1;
        let r = replies(&c.client(&Request::Range {
            req_id: *req,
            lo: Bound::Included(b"k/".to_vec()),
            hi: Bound::Excluded(b"l/".to_vec()),
            reverse: false,
            after: after.clone(),
            max_entries: 256,
        }));
        let Some((entries, cursor, at)) = r.into_iter().find_map(|x| match x {
            Reply::Page {
                entries,
                cursor,
                at,
                ..
            } => Some((entries, cursor, at)),
            _ => None,
        }) else {
            panic!("no page")
        };
        n += entries.len();
        match cursor {
            Some(c2) => after = Some(c2),
            None => return (n, at.root),
        }
    }
}

#[test]
#[ignore = "measurement: cargo test --release -p testkit --test zz_measure_read_state -- --ignored --nocapture"]
fn measure_design_b_costs() {
    for rows in [1_000usize, 10_000] {
        let node = PageNode::new();
        let mut x = node.connect();
        x.client(&Request::Identity);
        let mut id = 0u64;
        for chunk in 0..(rows / 100) {
            id += 1;
            let ops = (0..100)
                .map(|i| {
                    Op::Put(
                        format!("k/{:06}", chunk * 100 + i).into_bytes(),
                        vec![7u8; 64],
                    )
                })
                .collect();
            x.client(&Request::Write { write_id: id, ops });
        }
        for t in 1..=5u64 {
            x.tick_at(x.now_ms() + t * 1000);
        }
        let mut y = node.connect();
        y.client(&Request::Identity);
        let mut req = 0u64;
        let g0 = y.served(Served::Get);
        let t0 = Instant::now();
        let (n, r0) = scan_all(&mut y, &mut req);
        let cold = t0.elapsed();
        let g1 = y.served(Served::Get);
        let t1 = Instant::now();
        let (n2, _) = scan_all(&mut y, &mut req);
        let warm = t1.elapsed();
        let g2 = y.served(Served::Get);
        // Another tab changes ONE row; y adopts the new head.
        id += 1;
        x.client(&Request::Write {
            write_id: id,
            ops: vec![Op::Put(
                format!("k/{:06}", rows / 2).into_bytes(),
                vec![9u8; 64],
            )],
        });
        for t in 1..=5u64 {
            x.tick_at(x.now_ms() + t * 1000);
        }
        // Until y ADOPTS x's head — measured, not assumed (a run that read the
        // same root twice measured nothing).
        let target = node.head().expect("x's head").1;
        let mut ticks = 0;
        loop {
            ticks += 1;
            // What the head subscription's HeadChanged does on a real node.
            y.with_server(|s| s.head_hint());
            y.tick_at(y.now_ms() + 1000);
            let (_, r) = scan_all(&mut y, &mut 1_000_000);
            if r == target || ticks > 60 {
                break;
            }
        }
        let g3 = y.served(Served::Get);
        let t2 = Instant::now();
        let (n3, r1) = scan_all(&mut y, &mut req);
        let after_move = t2.elapsed();
        assert_ne!(
            r1, r0,
            "y never adopted x's head in {ticks} ticks, so this measured one tree twice"
        );
        let g4 = y.served(Served::Get);
        // The diff LIVE would use to learn WHICH keys changed: old root → new.
        req += 1;
        let t3 = Instant::now();
        let d = replies(&y.client(&Request::ChangesSince {
            req_id: req,
            from: r0,
            lo: Bound::Included(b"k/".to_vec()),
            hi: Bound::Excluded(b"l/".to_vec()),
            max_entries: 256,
        }));
        let diff = t3.elapsed();
        let g5 = y.served(Served::Get);
        let changes = d.iter().find_map(|x| match x {
            Reply::Delta { changes, .. } => Some(changes.len()),
            _ => None,
        });
        println!("rows={rows}: cold full read {n} rows in {cold:?}, {} GETs | warm {n2} rows in {warm:?}, {} GETs | after ONE row changed elsewhere (root moved {}→{}): {n3} rows in {after_move:?}, {} GETs (+{} during adopt) | diff old→new root: {changes:?} change(s) in {diff:?}, {} GETs",
            g1 - g0, g2 - g1, hex4(&r0), hex4(&r1), g4 - g3, g3 - g2, g5 - g4);
    }
}
fn hex4(r: &[u8; 32]) -> String {
    engine::short_id(r)
}

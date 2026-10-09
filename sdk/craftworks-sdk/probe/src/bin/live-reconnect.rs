//! LIVE: DOES AN OPEN PAGE KEEP WORKING AFTER ITS SOCKET IS REPLACED? freenet 0.2.138 cleans up a disconnected
//! client (client_events.rs, the `ClientRequest::Disconnect` arm, sent synthetically by websocket.rs's
//! `notify_disconnect` when a socket closes): it drops that client's SUBSCRIPTIONS and its delegate NOTIFICATION
//! ROUTING (`delegate_app_registry::remove_client`). It does not uninstall a delegate (RegisterDelegate is an install,
//! "not an app conversation"). So after a reconnect a page must re-subscribe its head itself; its signer should still
//! answer. This probe checks both on a private node, through the page's own path (`PageIo`, frames byte for byte).
//!
//! Two pages on one node and one key: P (the page under test) and Q ("another client": the same identity on a second
//! connection). Per arm, on a FRESH node:
//! 1. P writes r/0; Q opens and reads it (the setup).
//! 2. P's socket is CLOSED (the node is not touched) and a new one opened.
//!    - Arm `reconnected`: P is told (`PageIo::reconnected`), as js/session.js does on a re-open.
//!    - Arm `control` (NEGATIVE CONTROL): P is NOT told. Its head subscription went with the old socket, so it must
//!      NOT see Q's write inside the window. If it does, this probe cannot see a lost subscription, and it says so.
//! 3. (a) Q writes r/1. Does P's subscription deliver the move, and does P read r/1, within WINDOW?
//! 4. (b) P writes r/2 (signed by the node's signer, landed as P's head). Does it reach Published, and does Q read it?
//!
//! Exit status is the verdict: 0 = the `reconnected` arm passed (a) and (b), AND the control did not see (a);
//! 1 = a defect (named); 2 = could not judge.
//! usage: PAGE_PORT=<port> PAGE_TMP=<dir> live-reconnect <signer.wasm> <block.wasm> <register.wasm>
use anyhow::{bail, Context, Result};
use futures::SinkExt;
use page::Ms;
use page_io::PageIo;
use probe::live::{now_ms, Sock};
use probe::node::{Mode, Node, TempTree};
use protocol::{Reply, Request};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

/// How long a page is given to see the other client's write. The page's own backstop re-read of the head is 120 s, so
/// a window well under it separates "the subscription delivered" from "the backstop found it".
const WINDOW: Duration = Duration::from_secs(30);

struct Client {
    sock: Sock,
    io: PageIo,
    session: u64,
    next_id: u64,
}


impl Client {
    /// Drive until `done`, or `budget` passes (then `Ok(None)`: not within it, a data point, not an error).
    async fn drive(
        &mut self,
        t0: Instant,
        budget: Duration,
        mut done: impl FnMut(&[Reply], &PageIo) -> bool,
    ) -> Result<Option<Vec<Reply>>> {
        let start = Instant::now();
        let mut replies = Vec::new();
        loop {
            for f in self.io.take_frames() {
                self.sock
                    .send(Message::Binary(f.into()))
                    .await
                    .context("send")?;
            }
            for r in self.io.take_replies() {
                replies.push(
                    protocol::decode_reply(&r)
                        .map_err(|d| anyhow::anyhow!("a reply that does not decode: {d:?}"))?,
                );
            }
            if done(&replies, &self.io) {
                return Ok(Some(replies));
            }
            if start.elapsed() > budget {
                return Ok(None);
            }
            if let Some(b) = probe::live::next_frame(&mut self.sock, &mut self.io, t0).await? {
                self.io.inbound(&b, Ms(now_ms(t0)));
            }
        }
    }

    async fn must(
        &mut self,
        t0: Instant,
        budget: Duration,
        what: &str,
        done: impl FnMut(&[Reply], &PageIo) -> bool,
    ) -> Result<Vec<Reply>> {
        self.drive(t0, budget, done).await?.with_context(|| {
            format!(
                "{what}: not within {budget:?}; unusable: {:?}",
                self.io.unusable()
            )
        })
    }

    fn send(&mut self, r: &Request) {
        self.io
            .client(&protocol::encode_session_request(4, self.session, r).expect("encodes"));
    }

    /// A forced one-row write, waited on until its outcome. `Ok(ms)` = Published.
    async fn write(&mut self, t0: Instant, key: &str) -> Result<std::result::Result<u128, String>> {
        self.next_id += 1;
        let id = self.next_id;
        let t = Instant::now();
        self.send(&Request::forced_write(
            id,
            vec![protocol::Op::Put(
                key.as_bytes().to_vec(),
                format!("value of {key}").into_bytes(),
            )],
        ));
        let got = self
            .drive(t0, Duration::from_secs(120), |r, _| {
                probe::verdict::write_outcome(id, r).is_some()
            })
            .await?;
        Ok(
            match got
                .as_deref()
                .and_then(|g| probe::verdict::write_outcome(id, g))
            {
                Some(Ok(())) => Ok(t.elapsed().as_millis()),
                Some(Err(why)) => Err(why),
                None => Err("no outcome within 120 s".into()),
            },
        )
    }

    /// The keys this page reads now (one Range page; the probe's trees are tiny).
    async fn keys(&mut self, t0: Instant) -> Result<Vec<String>> {
        self.next_id += 1;
        let rid = 10_000 + self.next_id;
        self.send(&Request::Range {
            req_id: rid,
            lo: protocol::Bound::Unbounded,
            hi: protocol::Bound::Unbounded,
            reverse: false,
            after: None,
            max_entries: 256,
        });
        let got = self
            .must(t0, Duration::from_secs(60), "a read", |rs, _| {
                rs.iter()
                    .any(|x| matches!(x, Reply::Page { req_id, .. } if *req_id == rid))
            })
            .await?;
        let Some(Reply::Page { entries, .. }) = got
            .into_iter()
            .find(|x| matches!(x, Reply::Page { req_id, .. } if *req_id == rid))
        else {
            bail!("no page")
        };
        Ok(entries
            .iter()
            .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
            .collect())
    }

    /// Wait up to `window` for this page to read `key`; re-read after each moment the page is idle. Returns the ms it
    /// took, or None.
    async fn sees(&mut self, t0: Instant, key: &str, window: Duration) -> Result<Option<u128>> {
        let t = Instant::now();
        while t.elapsed() < window {
            if self.keys(t0).await?.iter().any(|k| k == key) {
                return Ok(Some(t.elapsed().as_millis()));
            }
            // Let the page take in what the node pushes (a head move) before reading again.
            self.drive(t0, Duration::from_millis(500), |_, _| false)
                .await?;
        }
        Ok(None)
    }
}

async fn open(
    ws: &str,
    sk: &ed25519_dalek::SigningKey,
    a: &Artefacts,
    session: u64,
    t0: Instant,
) -> Result<Client> {
    let (sock, _) = tokio_tungstenite::connect_async(ws)
        .await
        .context("connecting")?;
    let mut c = Client {
        sock,
        io: probe::page::for_key(sk, &a.signer, a.block.clone(), a.register.clone()),
        session,
        next_id: 0,
    };
    c.must(
        t0,
        Duration::from_secs(60),
        "provisioning the signer",
        |_, io| io.provisioned(),
    )
    .await?;
    c.send(&Request::Identity);
    c.must(t0, Duration::from_secs(60), "identity", |r, _| {
        r.iter().any(|x| matches!(x, Reply::Identity { .. }))
    })
    .await?;
    Ok(c)
}

struct Artefacts {
    signer: Vec<u8>,
    block: Vec<u8>,
    register: Vec<u8>,
}

/// One arm on a fresh node. `tell`: whether P is told of its new socket.
async fn arm(ws: &str, a: &Artefacts, tell: bool) -> Result<serde_json::Value> {
    let t0 = Instant::now();
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let mut p = open(ws, &sk, a, 11, t0).await.context("P opens")?;
    p.write(t0, "r/0")
        .await?
        .map_err(|w| anyhow::anyhow!("setup: P's first write: {w}"))?;
    let mut q = open(ws, &sk, a, 12, t0).await.context("Q opens")?;
    q.sees(t0, "r/0", WINDOW)
        .await?
        .context("setup: Q does not read P's first write")?;
    let before = p.io.head_subscription();

    // THE SOCKET IS REPLACED: the old one closed (the node runs on), a new one opened.
    let mut old = std::mem::replace(
        &mut p.sock,
        tokio_tungstenite::connect_async(ws)
            .await
            .context("reconnecting")?
            .0,
    );
    old.close(None).await.ok();
    drop(old);
    if tell {
        p.io.reconnected(Ms(now_ms(t0)));
    }
    // Let the page send what a reconnect sends (the head re-read) before the other client writes.
    p.drive(t0, Duration::from_secs(5), |_, io| {
        io.head_subscription().answered
    })
    .await?;
    let resub = p.io.head_subscription();

    // (a) The other client writes; does P see it?
    let q_write = q.write(t0, "r/1").await?;
    let changes_at_q = p.io.head_subscription().changes;
    let seen = p.sees(t0, "r/1", WINDOW).await?;
    let after = p.io.head_subscription();

    // (b) P saves after the reconnect: signed, landed, and readable by Q.
    let p_write = p.write(t0, "r/2").await?;
    let q_reads = if p_write.is_ok() {
        q.sees(t0, "r/2", WINDOW).await?
    } else {
        None
    };

    Ok(serde_json::json!({
        "arm": if tell { "reconnected" } else { "control: not told" },
        "head_sub_before": format!("{before:?}"),
        "head_sub_after_new_socket": format!("{resub:?}"),
        "q_write_r1": format!("{q_write:?}"),
        "a_p_sees_r1_ms": seen,
        "a_head_moves_delivered_after_q_write": after.changes.saturating_sub(changes_at_q),
        "b_p_write_r2": format!("{p_write:?}"),
        "b_q_reads_r2_ms": q_reads,
        "p_unusable": format!("{:?}", p.io.unusable()),
        "window_s": WINDOW.as_secs(),
    }))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    std::process::exit(match run().await {
        Ok(v) => v,
        Err(e) => {
            println!(
                "{}",
                serde_json::json!({ "could_not_judge": format!("{e:#}") })
            );
            2
        }
    });
}

async fn run() -> Result<i32> {
    println!("freenet: {}", probe::node::freenet_version()?);
    let port: u16 = std::env::var("PAGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .context("PAGE_PORT=<port> is required; there is no default")?;
    probe::node::allowed_ports(&[port, port + 1]).context("PAGE_PORT")?;
    let tmp = std::env::var("PAGE_TMP")
        .context("PAGE_TMP=<dir> is required: the node's dirs go under it")?;
    let usage = "usage: live-reconnect <signer.wasm> <block.wasm> <register.wasm>";
    let mut args = std::env::args().skip(1);
    let a = Artefacts {
        signer: std::fs::read(args.next().context(usage)?)?,
        block: std::fs::read(args.next().context(usage)?)?,
        register: std::fs::read(args.next().context(usage)?)?,
    };
    probe::check(&a.signer)
        .map_err(|e| anyhow::anyhow!("the signer is refused by the import gate: {e}"))?;

    let mut out = Vec::new();
    for (i, tell) in [true, false].into_iter().enumerate() {
        let dir = std::path::PathBuf::from(&tmp)
            .join(format!("live-reconnect-{}-{i}", std::process::id()));
        let _tree = TempTree(dir.clone());
        let node = Node::spawn_in(
            port,
            &dir,
            Mode::IsolatedNetwork {
                network_port: port + 1,
            },
        )?;
        let r = arm(&node.ws(), &a, tell)
            .await
            .with_context(|| format!("arm {i}"))?;
        println!("{r}");
        out.push(r);
        drop(node);
    }
    let (live, control) = (&out[0], &out[1]);
    if !control["a_p_sees_r1_ms"].is_null() {
        println!("COULD NOT JUDGE: the control (P not told of its new socket) still saw the other client's write, so this probe cannot see a lost subscription");
        return Ok(2);
    }
    let a_ok = !live["a_p_sees_r1_ms"].is_null();
    let b_ok = !live["b_q_reads_r2_ms"].is_null();
    println!("VERDICT: (a) a reconnected page sees another client's write: {}; (b) it saves (signed, landed, read by the other): {}", if a_ok { "YES" } else { "NO" }, if b_ok { "YES" } else { "NO" });
    Ok(if a_ok && b_ok { 0 } else { 1 })
}

//! HEALTH WATCH: reads the day's public health bag (`health YYYY-MM-DD`, written by every page's `index.health`)
//! through a node, and prints the problems grouped by kind and space — newest first, with how many devices said it.
//!
//!   health-watch <ws url> <contracts/bag.wasm> [YYYY-MM-DD …]      (no date: today and yesterday, UTC)
//!   health-watch <ws url> <contracts/bag.wasm> --post '<json>'      (a report, as a page's `index.health` writes it)
use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use sha2::Digest;
use std::collections::BTreeMap;
use tokio_tungstenite::tungstenite::Message;
use wire::{unframe, Incoming, Reassembler};
use node_io::{get};

/// The bag a page's `index.point(ref)` writes: `inbox_address(sha256("craftworks pointers " + ref))`.
fn pointer_address(r: &str) -> [u8; 32] {
    let h: [u8; 32] = sha2::Sha256::digest(format!("craftworks pointers {r}").as_bytes()).into();
    craftworks_identity::inbox_address(&h)
}



fn day(offset_days: i64) -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 + offset_days * 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        bail!("health-watch <ws url> <contracts/bag.wasm> [YYYY-MM-DD …]");
    }
    let bag_code = std::fs::read(&a[2]).context("bag.wasm")?;
    if a.get(3).map(String::as_str) == Some("--post") {
        let address = pointer_address(&format!("health {}", day(0)));
        let item = craftworks_bag_contract::grind(&address, a.get(4).context("the report's json")?.as_bytes());
        let state = craftworks_bag_contract::encode(&address, &[item]);
        let (_, c, _) = wire::puts::contract(&bag_code, &address, &[]);
        let (mut tx, mut rx) = tokio_tungstenite::connect_async(&a[1]).await?.0.split();
        for f in wire::frame_put(c, freenet_stdlib::prelude::WrappedState::new(state), 2).map_err(|e| anyhow::anyhow!(e))? {
            tx.send(Message::Binary(f.into())).await?;
        }
        let mut r = Reassembler::new();
        while let Some(Ok(Message::Binary(b))) = rx.next().await {
            match unframe(&mut r, &b) {
                Incoming::Ack(_) => return Ok(println!("posted")),
                Incoming::Partial => {}
                other => bail!("not posted: {other:?}"),
            }
        }
        bail!("no answer");
    }
    let days: Vec<String> = if a.len() > 3 { a[3..].to_vec() } else { vec![day(0), day(-1)] };
    let hm = |v: &serde_json::Value| v["at"].as_u64().map(|ms| { let s = ms / 1000 % 86_400; format!("{:02}:{:02}", s / 3600, s / 60 % 60) }).unwrap_or("--:--".into());
    for d in days {
        // The day's bags CHAINED (`health <day>`, `health <day> #1`, …: `index.health`), each full at ROOM items.
        let mut items = Vec::new();
        let mut found = false;
        for k in 0..64 {
            let address = pointer_address(&if k == 0 { format!("health {d}") } else { format!("health {d} #{k}") });
            let (_, c, _) = wire::puts::contract(&bag_code, &address, &[]);
            let id: [u8; 32] = c.key().id().as_bytes()[..32].try_into().unwrap();
            let Some(state) = get(&a[1], id).await? else { break };
            found = true;
            let these = craftworks_bag_contract::read(&address, &state).unwrap_or_default();
            let full = craftworks_bag_contract::chained::full(these.iter().map(|i| craftworks_bag_contract::payload(i).len()));
            items.extend(these);
            if !full {
                break;
            }
        }
        if !found {
            println!("{d}: no health bag (nothing reported, or not reachable)");
            continue;
        }
        let all: Vec<serde_json::Value> = items.iter().filter_map(|it| serde_json::from_slice(craftworks_bag_contract::payload(it)).ok()).collect();
        // HEARTBEATS (`seen`): each account's pages open that day — first and last beat, version, and each space's
        // state as it went (a change of a space's `on`/epoch shown in time order).
        let mut beats: BTreeMap<String, Vec<&serde_json::Value>> = BTreeMap::new();
        for v in all.iter().filter(|v| v["k"] == "seen") {
            beats.entry(v["r"].as_str().unwrap_or("?").to_string()).or_default().push(v);
        }
        println!("{d}: {} report(s) — {} heartbeat(s) from {} account(s)", all.len(), beats.values().map(Vec::len).sum::<usize>(), beats.len());
        for (r, mut vs) in beats {
            vs.sort_by_key(|v| v["at"].as_u64().unwrap_or(0));
            let last = vs[vs.len() - 1];
            println!("  seen {r}  ×{:<3} {}–{} UTC  v={}", vs.len(), hm(vs[0]), hm(last), last["v"].as_str().unwrap_or("?"));
            let mut was: BTreeMap<String, String> = BTreeMap::new();
            for v in &vs {
                for sp in v["sp"].as_array().into_iter().flatten() {
                    let s = sp["s"].as_str().unwrap_or("?").to_string();
                    let now = format!("e={} oe={} on={} unread={}{}", sp["e"], sp["oe"], sp["on"], sp["u"], sp.get("why").map(|w| format!(" why={w}")).unwrap_or_default());
                    if was.get(&s) != Some(&now) {
                        println!("    {} space {s:6} {now}", hm(v));
                        was.insert(s, now);
                    }
                }
            }
        }
        // PROBLEMS: by kind × space (× the member it is about), newest first.
        let mut groups: BTreeMap<(String, String, String), Vec<&serde_json::Value>> = BTreeMap::new();
        for v in all.iter().filter(|v| v["k"] != "seen") {
            let k = (v["k"].as_str().unwrap_or("?").to_string(), v["s"].as_str().unwrap_or("-").to_string(), v["to"].as_str().unwrap_or("").to_string());
            groups.entry(k).or_default().push(v);
        }
        for ((k, s, to), mut vs) in groups {
            vs.sort_by_key(|v| std::cmp::Reverse(v["at"].as_u64().unwrap_or(0)));
            let n = vs.len();
            let last = vs[0];
            // How many ACCOUNTS (`r`: the reporter's id prefix — none on reports from before it was sent).
            let mut who: Vec<&str> = vs.iter().filter_map(|v| v["r"].as_str()).collect();
            who.sort();
            who.dedup();
            let detail: Vec<String> = ["r", "st", "e", "oe", "why", "v"].iter().filter_map(|f| last.get(*f).filter(|x| !x.is_null()).map(|x| format!("{f}={}", x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())))).collect();
            let about = if to.is_empty() { String::new() } else { format!(" member {to}") };
            println!("  {k:15} space {s:6}{about}  ×{n:<3} from {} account(s)  newest {} UTC: {}", who.len(), hm(last), detail.join(" "));
            // What settles its cause (`d`), as the page measured it.
            if let Some(d) = last.get("d").filter(|x| !x.is_null()) {
                println!("  {:15}               detail: {d}", "");
            }
        }
    }
    Ok(())
}

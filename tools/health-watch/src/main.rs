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
    for d in days {
        let address = pointer_address(&format!("health {d}"));
        let (_, c, _) = wire::puts::contract(&bag_code, &address, &[]);
        let id: [u8; 32] = c.key().id().as_bytes()[..32].try_into().unwrap();
        let Some(state) = get(&a[1], id).await? else {
            println!("{d}: no health bag (nothing reported, or not reachable)");
            continue;
        };
        let items = craftworks_bag_contract::read(&address, &state).unwrap_or_default();
        let mut groups: BTreeMap<(String, String), Vec<serde_json::Value>> = BTreeMap::new();
        for it in &items {
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(craftworks_bag_contract::payload(it)) else { continue };
            let k = (v["k"].as_str().unwrap_or("?").to_string(), v["s"].as_str().unwrap_or("-").to_string());
            groups.entry(k).or_default().push(v);
        }
        println!("{d}: {} report(s), {} kind×space", items.len(), groups.len());
        for ((k, s), mut vs) in groups {
            vs.sort_by_key(|v| std::cmp::Reverse(v["at"].as_u64().unwrap_or(0)));
            let n = vs.len();
            let last = &vs[0];
            // How many ACCOUNTS (`r`: the reporter's id prefix — none on reports from before it was sent).
            let mut who: Vec<&str> = vs.iter().filter_map(|v| v["r"].as_str()).collect();
            who.sort();
            who.dedup();
            let detail: Vec<String> = ["r", "e", "oe", "to", "why", "v"].iter().filter_map(|f| last.get(*f).filter(|x| !x.is_null()).map(|x| format!("{f}={}", x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())))).collect();
            println!("  {k:15} space {s:6}  ×{n:<3} from {} account(s)  newest: {}", who.len(), detail.join(" "));
            // What settles its cause (`d`), as the page measured it.
            if let Some(d) = last.get("d").filter(|x| !x.is_null()) {
                println!("  {:15}               detail: {d}", "");
            }
        }
    }
    Ok(())
}

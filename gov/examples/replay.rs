//! The replay as a filter (the differential test against the page's former replay): stdin one JSON case per line
//! `{ rows: [[id, value, writer]], writers: {node: did}, first, dids: [..], codes: [..], times: [..] }`, stdout what the
//! governance says of it, one JSON line per case.
use craftworks_gov::{Gov, Row, ACTIONS};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::BufRead;

fn main() {
    for line in std::io::stdin().lock().lines() {
        let c: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let rows: Vec<Row> = c["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| Row { id: r[0].as_str().unwrap().into(), value: r[1].as_str().unwrap().into(), writer: r[2].as_str().map(str::to_string) })
            .collect();
        let writers: HashMap<String, String> = serde_json::from_value(c["writers"].clone()).unwrap();
        let g = Gov::replay(&rows, &writers, c["first"].as_str(), c["now"].as_f64().unwrap());
        let dids: Vec<&str> = c["dids"].as_array().unwrap().iter().map(|d| d.as_str().unwrap()).collect();
        let times: Vec<f64> = c["times"].as_array().unwrap().iter().map(|t| t.as_f64().unwrap()).collect();
        let paths = ["", "chat", "board", "board/p/1", "notes"];
        let mut eff = vec![];
        for p in paths {
            for a in ACTIONS {
                for t in &times {
                    eff.push(json!([p, a, t, g.effective(p, a, *t), g.policy_at(p, a, *t)]));
                }
            }
        }
        let mut invites = vec![];
        for t in &times {
            invites.push(json!(g.live_invites(*t).iter().map(|i| json!([i.code, i.by, i.admitted])).collect::<Vec<_>>()));
        }
        let apps: Vec<&str> = craftworks_gov::APPS.iter().copied().filter(|a| g.apps.get(*a) == Some(&true)).collect();
        let configs: Vec<Option<Value>> = ["board/rules", "board/read", "chat/post", "space/join"].iter().map(|k| g.configs.get(*k).cloned()).collect();
        let roles: Vec<Value> = dids.iter().map(|d| json!([g.role(d, false), g.role(d, true)])).collect();
        let counted: Vec<Value> = g.counted.iter().map(|a| a["id"].clone()).collect();
        let out = json!({
            "owner": g.owner,
            "roles": roles,
            "counted": counted,
            "roster": g.roster,
            "bans": g.bans,
            "apps": apps,
            "configs": configs,
            "eff": eff,
            "invites": invites,
            "public": g.is_public(),
        });
        println!("{out}");
    }
}

//! TURNS AND REQUESTS — what the page and the identity delegate share for acting on a group (FOUNDATION "Members and
//! keys", the design of 10-17): who does a thing first (`turn`), and where a request to join waits (`bag_name`).
//! Nobody is let into a group's keys by another member: a listed device enters by itself (`mls::enter`); the list is
//! decided by the space's acts (`gov`).

/// WHO DOES A THING (no claim, no message): every member ranks `dids` the same way for `id` — 0 does it now, k takes
/// it over after k × the takeover time; -1: `me` is not among them. FNV-1a over the text (the page's `ordering.turn`
/// is this function, through the core: one ranking).
pub fn turn(id: &str, dids: &[String], me: &str) -> i64 {
    let hash = |s: &str| -> u32 {
        let mut h: u32 = 2166136261;
        for c in s.encode_utf16() {
            h = (h ^ u32::from(c)).wrapping_mul(16777619);
        }
        h
    };
    let mut all: Vec<&String> = Vec::new();
    for d in dids {
        if !all.contains(&d) {
            all.push(d);
        }
    }
    all.sort_by(|a, b| hash(&format!("{id}|{b}")).cmp(&hash(&format!("{id}|{a}"))).then_with(|| a.cmp(b)));
    all.iter().position(|d| d.as_str() == me).map_or(-1, |i| i as i64)
}

/// A REQUEST BAG's NAME (the page's and the node's): a code's own (made before codes carried their space); a space's
/// requests `open <space hex>`.
pub fn bag_name(code: &str, space: &[u8; 32]) -> String {
    match code {
        "open" => format!("{code} {}", space.iter().map(|b| format!("{b:02x}")).collect::<String>()),
        _ => code.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_matches_the_pages_ordering() {
        // ordering.js: FNV-1a over UTF-16 code units, highest first, ties by name — the values below computed there.
        let dids: Vec<String> = ["did:craftec:a", "did:craftec:b", "did:craftec:c", "did:craftec:7kqYgwompGP2j5Q6n8TYDyY3ymYq2GaxbG86rHkv4nBz"].map(String::from).to_vec();
        let ranks = |id: &str| dids.iter().map(|d| turn(id, &dids, d)).collect::<Vec<_>>();
        assert_eq!(ranks("k"), vec![0, 1, 2, 3]);
        assert_eq!(ranks("admit|did:x|999000"), vec![3, 1, 2, 0]);
        assert_eq!(turn("k", &dids, "did:craftec:z"), -1);
    }
}

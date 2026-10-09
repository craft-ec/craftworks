//! THE NODE ABOUT ITSELF (storage proofs, TOKEN §7a): a page asks its node for its peer id, its ring location and
//! every contract it hosts, and for its connected peers — answers built here as the stdlib's own types, encoded as the
//! node encodes them, and read back through `unframe` (chunked, as a node with many contracts sends it).

use freenet_stdlib::client_api::streaming::chunk_response;
use freenet_stdlib::client_api::{ClientError, ContractState, HostResponse, NodeDiagnosticsResponse, NodeInfo, QueryResponse};
use freenet_stdlib::prelude::ContractInstanceId;
use wire::{millionths, unframe, Incoming, Reassembler};

fn frames(r: HostResponse, chunked: bool) -> Vec<Vec<u8>> {
    let whole = bincode::serialize(&Ok::<HostResponse, ClientError>(r)).unwrap();
    if !chunked {
        return vec![whole];
    }
    chunk_response(whole, 7).into_iter().map(|c| bincode::serialize(&Ok::<HostResponse, ClientError>(c)).unwrap()).collect()
}
fn read(frames: Vec<Vec<u8>>) -> Incoming {
    let mut r = Reassembler::new();
    let mut last = Incoming::Partial;
    for f in frames {
        last = unframe(&mut r, &f);
    }
    last
}

#[test]
fn a_location_reads_exactly_in_millionths() {
    assert_eq!(millionths("0.596280"), Some(596_280));
    assert_eq!(millionths("0.5"), Some(500_000));
    assert_eq!(millionths("1.000000"), Some(1_000_000));
    for bad in ["", "0", "0.", "1.5", "2.0", "0.1234567", "-0.1", "0.12a", ".5"] {
        assert_eq!(millionths(bad), None, "{bad:?}");
    }
}

#[test]
fn the_node_names_itself_its_location_and_every_contract_it_hosts() {
    // Many contracts, so the answer is chunked as a real node's is (25,635 on fx, 10-09).
    let ids: Vec<[u8; 32]> = (0..20_000u32).map(|i| *blake3::hash(&i.to_le_bytes()).as_bytes()).collect();
    let d = NodeDiagnosticsResponse {
        node_info: Some(NodeInfo { peer_id: "5cCFQqQwLPNKX4zwK".into(), is_gateway: true, location: Some("0.596280".into()), listening_address: None, uptime_seconds: 1 }),
        network_info: None,
        subscriptions: vec![],
        contract_states: ids
            .iter()
            .enumerate()
            .map(|(i, id)| (ContractInstanceId::new(*id).to_string(), ContractState { subscribers: 0, subscriber_peer_ids: vec![], size_bytes: i as u64 }))
            .chain([("not base58!".to_string(), ContractState { subscribers: 0, subscriber_peer_ids: vec![], size_bytes: 9 })])
            .collect(),
        system_metrics: None,
        connected_peers_detailed: vec![],
    };
    let f = frames(HostResponse::QueryResponse(QueryResponse::NodeDiagnostics(d)), true);
    assert!(f.len() > 1, "THE SETUP: the answer was not chunked");
    match read(f) {
        Incoming::NodeSelf { peer_id, location, hosted } => {
            assert_eq!(peer_id, "5cCFQqQwLPNKX4zwK");
            assert_eq!(location, Some(596_280));
            assert_eq!(hosted.len(), ids.len(), "every contract, the unreadable name left out");
            let mut want: Vec<([u8; 32], u64)> = ids.iter().enumerate().map(|(i, id)| (*id, i as u64)).collect();
            want.sort();
            assert_eq!(hosted, want, "each id with its size, sorted");
        }
        other => panic!("not the node's answer about itself: {other:?}"),
    }
}

#[test]
fn the_node_names_its_peers_and_the_address_it_sees() {
    let peers: Vec<(String, String)> = match read(frames(probe_peers(), false)) {
        Incoming::Peers(p) => p,
        other => panic!("not peers: {other:?}"),
    };
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].1, "127.0.0.1:37692");
}

/// A `ConnectedPeers` answer as fx sent it (its bytes, captured 10-09 through the same stdlib).
fn probe_peers() -> HostResponse {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/connected_peers.bin")).expect("the captured answer");
    match bincode::deserialize::<Result<HostResponse, ClientError>>(&bytes).expect("decodes") {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    }
}

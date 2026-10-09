//! WHICH SITE A PATH OR AN ADDRESS NAMES (sdk#399 step 4's loader handover; sdk#472's keepSet): exactly the 32-byte
//! id of `/v1/contract/web/<link>/`, a bare `<link>`, or a node URL around it -- or a refusal by name. The controls are
//! the forms a lenient decode turns into a well-formed WRONG id.
use page_io::{site_id_of_address, site_id_of_path, site_text, AddressRefused as R};

fn link(id: [u8; 32]) -> String {
    freenet_stdlib::prelude::ContractInstanceId::new(id).encode()
}

#[test]
fn a_path_names_its_site_exactly() {
    let id = [7u8; 32];
    let l = link(id);
    for path in [format!("/v1/contract/web/{l}/"), format!("/v1/contract/web/{l}/index.html"), format!("/v1/contract/web/{l}"), format!("/v1/contract/web/{l}?x=1"), format!("/v1/contract/web/{l}#top")] {
        assert_eq!(site_id_of_path(&path), Some(id), "{path}");
    }
    for (what, path) in [
        ("the round-trip trap (from_base58 zero-pads a short text into the all-zero id)", "/v1/contract/web/1/".to_string()),
        ("not base58", "/v1/contract/web/0OIl/".to_string()),
        ("no link", "/v1/contract/web//".to_string()),
        ("another path", format!("/v1/contract/{l}/")),
        ("a dev server's page", "/index.html".to_string()),
        ("nothing", String::new()),
    ] {
        assert_eq!(site_id_of_path(&path), None, "{what} was taken for a site");
    }
}

#[test]
fn an_address_is_a_bare_link_a_node_url_or_the_path() {
    let id = [9u8; 32];
    let l = link(id);
    for addr in [
        l.clone(),
        format!("  {l}  "),
        format!("http://127.0.0.1:7509/v1/contract/web/{l}/"),
        format!("https://node.example:443/v1/contract/web/{l}/app/index.html?q=1#frag"),
        format!("http://localhost/v1/contract/web/{l}?q=1"),
        format!("http://localhost/v1/contract/web/{l}#frag"),
        format!("/v1/contract/web/{l}/index.html"),
    ] {
        assert_eq!(site_id_of_address(&addr), Ok(id), "{addr}");
    }
}

#[test]
fn anything_else_is_refused_by_name() {
    let l = link([9u8; 32]);
    for (addr, why) in [
        (String::new(), R::Empty),
        ("   ".to_string(), R::Empty),
        ("1".to_string(), R::NotASiteLink),
        ("http://127.0.0.1:7509/v1/contract/web/1/".to_string(), R::NotASiteLink),
        ("0OIl".to_string(), R::NotASiteLink),
        (format!("craftec://{l}"), R::UnknownScheme),
        (format!("ftp://h/v1/contract/web/{l}/"), R::UnknownScheme),
        (format!("http://127.0.0.1:7509/v1/contract/{l}/"), R::NotASitePath),
        ("http://127.0.0.1:7509".to_string(), R::NotASitePath),
        (format!("{l}/index.html"), R::NotASitePath),
        (format!("{l}?q=1"), R::NotASitePath),
        ("/index.html".to_string(), R::NotASitePath),
    ] {
        assert_eq!(site_id_of_address(&addr), Err(why), "{addr:?}");
    }
}

#[test]
fn site_text_is_the_parsers_inverse() {
    for id in [[0u8; 32], [9u8; 32], [255u8; 32]] {
        let text = site_text(&id);
        assert_eq!(text, link(id), "not the node's own encoding");
        assert_eq!(site_id_of_address(&text), Ok(id), "{text} does not parse back");
        assert_eq!(site_id_of_path(&format!("/v1/contract/web/{text}/")), Some(id));
    }
}

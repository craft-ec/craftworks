//! THE LOADER'S POINTER CHECK (ARCHITECTURE §19, the bootstrap; app-as-data P5): a site holds the starter and a
//! `pointer.json` naming whose tree the app is in. The loader reads it through ONE owner of its bytes
//! (`wire::webapp::read_site_pointer`, the inverse of `site_pointer`), recomputes the site link from it and REFUSES a
//! pointer that is not the page's own: a pointer copied from another site names another link.

use page_io::{open_pointer, site_contract, PointerRefused};

const SITE_CODE: &[u8] = b"site-pointer test site code";
const REGISTER_CODE: &[u8] = b"site-pointer test register code";

fn params(seed: u8) -> Vec<u8> {
    let sk = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME)
}

/// The path a page of `app`'s site (published under `params`) is served at.
fn page_of(params: &[u8], app: &str) -> String {
    let c = site_contract(SITE_CODE, params, app).expect("a site contract");
    format!("/v1/contract/web/{}/index.html", c.key().id().encode())
}

#[test]
fn a_pointer_reads_back_exactly_as_composed_and_a_look_alike_is_none() {
    let p = params(1);
    let bytes = wire::webapp::site_pointer(&p, "notes").expect("a pointer");
    assert_eq!(wire::webapp::read_site_pointer(&bytes), Some((p.clone(), "notes".to_string())));
    let text = String::from_utf8(bytes.clone()).expect("utf-8");
    for look_alike in [
        text.replace(r#"{"app":"#, r#"{ "app":"#),
        text.replace(r#"}"#, r#","extra":1}"#),
        format!(r#"{{"register_params":"{}","app":"notes"}}"#, core_types::hex::encode(&p)),
        text.replace("notes", "No Tes"),
        text.to_uppercase(),
        String::new(),
    ] {
        assert_eq!(wire::webapp::read_site_pointer(look_alike.as_bytes()), None, "read as a pointer: {look_alike}");
    }
}

#[test]
fn the_pages_own_pointer_opens_and_names_the_owners_register() {
    let p = params(2);
    let pointer = wire::webapp::site_pointer(&p, "notes").expect("a pointer");
    let (app, register) = open_pointer(&pointer, &page_of(&p, "notes"), SITE_CODE, REGISTER_CODE).expect("the page's own pointer");
    assert_eq!(app, "notes");
    assert_eq!(register, signer::register_id(REGISTER_CODE, &p), "not the owner's Register");
    // THE CONTROL: the same pointer under another Register code names another Register, so the id is derived, not fixed.
    assert_ne!(register, signer::register_id(b"another register code", &p));
}

#[test]
fn a_swapped_pointer_is_refused_whoever_or_whichever_app_it_names() {
    let (mine, theirs) = (params(3), params(4));
    let page = page_of(&mine, "notes");
    for (who, pointer) in [
        ("another person's pointer", wire::webapp::site_pointer(&theirs, "notes").expect("a pointer")),
        ("another app's pointer", wire::webapp::site_pointer(&mine, "tasks").expect("a pointer")),
    ] {
        match open_pointer(&pointer, &page, SITE_CODE, REGISTER_CODE) {
            Err(PointerRefused::NotThisSite { names, page: at }) => assert_ne!(names, at, "{who}"),
            other => panic!("{who} was not refused as another site's: {other:?}"),
        }
    }
    // THE CONTROL: the page's own pointer, on the same page, opens.
    assert!(open_pointer(&wire::webapp::site_pointer(&mine, "notes").expect("a pointer"), &page, SITE_CODE, REGISTER_CODE).is_ok());
}

#[test]
fn a_page_off_a_site_path_or_a_non_pointer_is_refused_by_name() {
    let p = params(5);
    let pointer = wire::webapp::site_pointer(&p, "notes").expect("a pointer");
    assert_eq!(open_pointer(&pointer, "/index.html", SITE_CODE, REGISTER_CODE), Err(PointerRefused::NotASitePath));
    assert_eq!(open_pointer(b"{}", &page_of(&p, "notes"), SITE_CODE, REGISTER_CODE), Err(PointerRefused::NotAPointer));
}

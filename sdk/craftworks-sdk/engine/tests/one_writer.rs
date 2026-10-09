//! ONE WRITER, BY TYPE (WANTED-LIFE, sdk#480 part 3; CLAUDE.md "One writer, by type"): the three reader indexes live
//! in `engine::wanted::Wanted`, whose fields are PRIVATE to that module, so the compiler refuses any write outside its
//! four transitions -- no list of spellings to walk around (the architect on #504). What the compiler cannot hold is
//! the TYPE ITSELF being opened up, so this test holds that: every field of `Wanted` stays private, and no other engine
//! source declares a reader index of its own.

const WANTED: &str = include_str!("../src/wanted.rs");
const OTHERS: [(&str, &str); 3] = [
    ("src/lib.rs", include_str!("../src/lib.rs")),
    ("src/read.rs", include_str!("../src/read.rs")),
    ("src/race_get.rs", include_str!("../src/race_get.rs")),
];

/// The fields of `pub(crate) struct Wanted { .. }`, as written.
fn wanted_fields(src: &str) -> Vec<&str> {
    let start = src.find("pub(crate) struct Wanted {").expect("THE CONTROL: `struct Wanted` not found in wanted.rs");
    let body = &src[start..src[start..].find("\n}\n").map(|e| start + e).expect("the struct's end")];
    body.lines().skip(1).map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).collect()
}

#[test]
fn the_reader_indexes_are_private_to_their_one_type() {
    let fields = wanted_fields(WANTED);
    // THE CONTROL: the three indexes are the fields found.
    for name in ["waiting:", "slots:", "needs:"] {
        // By its NAME, whatever its visibility: the control finds the field, the assertion below judges it.
        let named = |f: &&str| f.trim_start_matches("pub(crate) ").trim_start_matches("pub ").starts_with(name);
        assert!(fields.iter().any(named), "THE CONTROL: `Wanted` has no `{name}` field: {fields:?}");
    }
    let opened: Vec<&&str> = fields.iter().filter(|f| f.starts_with("pub")).collect();
    assert!(opened.is_empty(), "a field of `Wanted` is not private -- code outside its four transitions could write it: {opened:?}");
    // No second home: another engine source declaring a block -> readers map or a write's needs set of its own.
    for (path, src) in OTHERS {
        for (n, line) in src.lines().enumerate() {
            let l = line.trim();
            if l.starts_with("//") {
                continue;
            }
            let second = l.starts_with("waiting: BTreeMap<Cid") || l.starts_with("pub waiting:") || l.starts_with("repair_slots:") || l.starts_with("needs: BTreeSet<Cid>");
            assert!(!second, "{path}:{}: a reader index declared outside `Wanted`: {l}", n + 1);
        }
    }
}

/// The reader's own control: a `pub` field is seen, and a private one is not flagged.
#[test]
fn the_field_reader_sees_a_pub_field() {
    let open = "pub(crate) struct Wanted {\n    pub(crate) waiting: X,\n    slots: Y,\n}\n";
    assert_eq!(wanted_fields(open), vec!["pub(crate) waiting: X,", "slots: Y,"]);
}

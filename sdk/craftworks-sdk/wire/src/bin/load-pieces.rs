//! Cut a published app's LOAD BUNDLE into its k + m piece containers (sdk#347).
//!
//! usage: load-pieces <webapp.wasm> <payload> <m> <out dir> <name=path | path>...
//!
//! The files (each named by its `name=` if given, else its base name; a name may be a path like `sdk/index.js`,
//! which is how a module's relative imports resolve inside the bundle) become one bundle (`pieces::bundle`), cut into k data pieces of at most
//! <payload> bytes and <m> parity pieces (`pieces::cut`, the SDK's one implementation). Each piece becomes its OWN
//! web container holding one file, `piece`, under the `webapp` contract, so a node serves it at
//! `/v1/contract/web/<address>/piece`. Writes, per piece i: <out>/piece-<i>.bin (the served bytes, for the caller to
//! hash) and <out>/piece-<i>.webapp (the container state to PUT); and <out>/bundle.bin. Prints one JSON object:
//! `{ k, m, payload, bundle_len, pieces: [ { address } ] }`; the caller adds each piece's sha256 from its .bin.
fn main() -> Result<(), String> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: load-pieces <webapp.wasm> <payload> <m> <out dir> <name=path | path>...";
    if a.len() < 5 {
        return Err(usage.into());
    }
    let code = std::fs::read(&a[0]).map_err(|e| format!("{}: {e}", a[0]))?;
    let payload: usize = a[1].parse().map_err(|_| usage)?;
    let m: usize = a[2].parse().map_err(|_| usage)?;
    let out = &a[3];
    let mut files = Vec::new();
    for arg in &a[4..] {
        let (name, p) = match arg.split_once('=') {
            Some((n, p)) => (n.to_string(), p.to_string()),
            None => (std::path::Path::new(arg).file_name().and_then(|n| n.to_str()).ok_or(format!("{arg}: no file name"))?.to_string(), arg.clone()),
        };
        if name.is_empty() || name.starts_with('/') || name.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
            return Err(format!("{name}: a bundle name is a plain relative path"));
        }
        let p = &p;
        let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
        if name.ends_with(".js") {
            if let Some(line) = dynamic_import(&String::from_utf8_lossy(&bytes)) {
                return Err(format!(
                    "{name}:{line}: a dynamic import(...) of a non-literal: the page links the bundle's modules by rewriting each \
                     LITERAL relative specifier (linkModules), and cannot rewrite this one"
                ));
            }
        }
        files.push((name, bytes));
    }
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let bundle = pieces::bundle(&refs).map_err(|e| format!("{e:?}"))?;
    let cut = pieces::cut(&bundle, payload, m).map_err(|e| format!("{e:?}"))?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(format!("{out}/bundle.bin"), &bundle).map_err(|e| e.to_string())?;
    let mut addrs = Vec::new();
    for (i, p) in cut.pieces.iter().enumerate() {
        let state = wire::webapp::piece_container(p)?;
        addrs.push(wire::webapp::address(&code, &state));
        std::fs::write(format!("{out}/piece-{i}.bin"), p).map_err(|e| e.to_string())?;
        std::fs::write(format!("{out}/piece-{i}.webapp"), &state).map_err(|e| e.to_string())?;
    }
    let list = addrs.iter().map(|a| format!("{{ \"address\": \"{a}\" }}")).collect::<Vec<_>>().join(", ");
    println!("{{ \"k\": {}, \"m\": {}, \"payload\": {}, \"bundle_len\": {}, \"pieces\": [{list}] }}", cut.k, cut.m, cut.payload, cut.bundle_len);
    Ok(())
}

/// A MODULE THE PAGE CAN LINK (sdk#347): a published app's loader links the bundle's ES modules by rewriting each
/// relative import's LITERAL specifier to that module's URL (`linkModules`). A dynamic `import(expr)` names nothing
/// it can rewrite, so it would resolve against a blob: URL at run time and fail there; it is refused HERE, at build.
/// The first line holding one (1-based), skipping comment lines.
fn dynamic_import(text: &str) -> Option<usize> {
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('*') || t.starts_with("/*") {
            continue;
        }
        let mut rest = line;
        while let Some(at) = rest.find("import") {
            let after = rest[at + "import".len()..].trim_start();
            let word_start = at == 0 || !rest[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '$' || c == '.');
            if word_start && after.starts_with('(') && !after[1..].trim_start().starts_with(['"', '\'']) {
                return Some(i + 1);
            }
            rest = &rest[at + "import".len()..];
        }
    }
    None
}

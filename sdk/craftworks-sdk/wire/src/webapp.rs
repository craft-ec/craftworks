//! The ARTEFACTS web container (craftworks-builder#104): the SDK build's own
//! artefacts, framed as freenet serves a web container, under the `webapp`
//! contract (freenet-contracts#47).
//!
//! An app names its artefacts by hash and carries none of them (ARCHITECTURE
//! §19). This is where they live: one container per SDK BUILD, the same bytes
//! for every app of that build, so it is PUT once and every later PUT of it is
//! the same contract. It ships in `pkg/web` beside the files it holds, with its
//! address in `artefacts.json` — builders never rebuild it, which is why its
//! bytes need to be reproducible per toolchain (like the wasm), not across
//! machines.
//!
//! - [`tar`]: a deterministic ustar — sorted paths, mode 0644, uid/gid 0,
//!   mtime 0, no owner names — so the same files give the same bytes.
//! - [`container`]: the node's framing, `[u64 BE meta len][meta][u64 BE web
//!   len][web]`, exactly what `webapp` validates.
//! - [`address`]: the contract instance id the node serves it under, derived by
//!   freenet-stdlib itself from the `webapp` code and `params = blake3(state)`.

use freenet_stdlib::prelude::*;

const BLOCK: usize = 512;

/// A deterministic ustar archive of `files`, sorted by path whatever order
/// they are given in. Regular files only; a path must fit ustar's 100 bytes.
pub fn tar(files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    let mut sorted: Vec<&(&str, &[u8])> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = Vec::new();
    for (path, body) in sorted {
        let name = path.as_bytes();
        if name.is_empty() || name.len() > 100 || path.contains("..") || path.starts_with('/') {
            return Err(format!("{path:?} is not a plain relative path of at most 100 bytes"));
        }
        let mut h = [0u8; BLOCK];
        h[..name.len()].copy_from_slice(name);
        let octal = |h: &mut [u8; BLOCK], at: usize, width: usize, v: u64| {
            let s = format!("{v:0w$o}\0", w = width - 1);
            h[at..at + width].copy_from_slice(s.as_bytes());
        };
        octal(&mut h, 100, 8, 0o644); // mode
        octal(&mut h, 108, 8, 0); // uid
        octal(&mut h, 116, 8, 0); // gid
        octal(&mut h, 124, 12, body.len() as u64); // size
        octal(&mut h, 136, 12, 0); // mtime
        h[156] = b'0'; // a regular file
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        // The checksum is summed with its own field as eight spaces.
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        let s = format!("{sum:06o}\0 ");
        h[148..156].copy_from_slice(s.as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(body);
        out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
    }
    out.resize(out.len() + 2 * BLOCK, 0);
    Ok(out)
}

/// The node's web-container framing (freenet 0.2.136 `app_packaging.rs`): the ONE statement of it is
/// `contract_keys::site::frame` (a site's state is the same framing, builder#117).
pub fn container(metadata: &[u8], web: &[u8]) -> Vec<u8> {
    contract_keys::site::frame(metadata, web)
}

/// `data` as a valid `.xz` stream whose one block holds LZMA2 STORED chunks:
/// no compression, so no encoder, only framing — CRC32 check, as the xz
/// format (1.2.0) specifies it. What a page can make without an xz encoder,
/// for an APP container (a few hundred KB of JavaScript, where the artefacts
/// container's native xz is what saves the megabytes). Deterministic: the
/// same data gives the same bytes.
pub fn xz_stored(data: &[u8]) -> Vec<u8> {
    const MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
    const FLAGS: [u8; 2] = [0x00, 0x01]; // check = CRC32
    let mut out = Vec::with_capacity(data.len() + data.len() / 65_536 * 3 + 64);
    // Stream header.
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FLAGS);
    out.extend_from_slice(&crc32(&FLAGS).to_le_bytes());
    // Block header: size byte ((12 / 4) - 1), flags (one filter, no sizes),
    // the LZMA2 filter (id 0x21, one property byte: the smallest dictionary,
    // which stored chunks never use), padding to 4, CRC32.
    let mut header = vec![0x02, 0x00, 0x21, 0x01, 0x00, 0x00, 0x00, 0x00];
    let hcrc = crc32(&header);
    header.extend_from_slice(&hcrc.to_le_bytes());
    out.extend_from_slice(&header);
    // LZMA2: uncompressed chunks of at most 64 KiB — the first resets the
    // dictionary (0x01), the rest do not (0x02) — then the end marker.
    let start = out.len();
    for (i, chunk) in data.chunks(65_536).enumerate() {
        out.push(if i == 0 { 0x01 } else { 0x02 });
        out.extend_from_slice(&((chunk.len() - 1) as u16).to_be_bytes());
        out.extend_from_slice(chunk);
    }
    out.push(0x00);
    let compressed = out.len() - start;
    out.resize(out.len().div_ceil(4) * 4, 0);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    // Index: one record (unpadded block size, uncompressed size), padded, CRC32.
    let mut index = vec![0x00];
    varint(&mut index, 1);
    varint(&mut index, (header.len() + compressed + 4) as u64);
    varint(&mut index, data.len() as u64);
    index.resize(index.len().div_ceil(4) * 4, 0);
    let icrc = crc32(&index);
    index.extend_from_slice(&icrc.to_le_bytes());
    out.extend_from_slice(&index);
    // Stream footer: CRC32 of (backward size, flags), backward size, flags, magic.
    let mut tail = ((index.len() / 4 - 1) as u32).to_le_bytes().to_vec();
    tail.extend_from_slice(&FLAGS);
    out.extend_from_slice(&crc32(&tail).to_le_bytes());
    out.extend_from_slice(&tail);
    out.extend_from_slice(b"YZ");
    out
}

/// An APP web container: `files` as a deterministic tar, stored-chunk xz,
/// in the node's framing, with no metadata.
pub fn app_container(files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    Ok(container(&[], &app_web(files)?))
}

/// The ONE file a LOAD PIECE's web container holds (sdk#347): a node serves it at `/v1/contract/web/<address>/piece`.
pub const PIECE_FILE: &str = "piece";

/// A LOAD PIECE's web container state (sdk#347, sdk#493): the ONE owner of "a piece -> its container". The build
/// (`load-pieces`), the loader's repair after load (`js/pieces.js`, through `sdk.pieces.container`) and the keeper's
/// repair all call it, so a rebuilt piece is framed exactly as it was published and lands at the address the
/// starter names (the one-home test counts every other framing of `PIECE_FILE`).
pub fn piece_container(piece: &[u8]) -> Result<Vec<u8>, String> {
    app_container(&[(PIECE_FILE, piece)])
}

/// The path of a site's POINTER (ARCHITECTURE §19, the bootstrap): the one file of a site that names whose tree the
/// app is in -- the loader reads it, checks it against its own link, and reads the app from that tree.
pub const POINTER_PATH: &str = "pointer.json";

/// The path of the per-version app MANIFEST a site held before app-as-data (the builder's `app.json`): a site no
/// longer carries one (the app is data in its tree), so a starter bringing it is refused. Named here, beside
/// [`POINTER_PATH`], so the site's file names have one home; app-as-data P5 deletes it with `publish_site`.
pub const LEGACY_APP_JSON: &str = "app.json";

/// A site's POINTER, the ONE owner of its bytes (app-as-data P4): `{"app":"<app id>","register_params":"<hex>"}` --
/// no version, head or seq, so a platform upgrade writes it byte for byte the same. `None` for an app id that is not
/// one (`core_types::name::app_ok`: nothing in it needs escaping) or no register params.
pub fn site_pointer(register_params: &[u8], app: &str) -> Option<Vec<u8>> {
    if register_params.is_empty() || !core_types::name::app_ok(app) {
        return None;
    }
    Some(format!(r#"{{"app":"{app}","register_params":"{}"}}"#, core_types::hex::encode(register_params)).into_bytes())
}

/// A site's POINTER, READ: its register params and app id -- the inverse of [`site_pointer`], and the second half of
/// the ONE owner of its bytes. STRICT: only the exact bytes [`site_pointer`] composes are a pointer (re-composed and
/// compared), so a pointer has one spelling and a look-alike -- spacing, key order, an extra field -- is `None`.
pub fn read_site_pointer(bytes: &[u8]) -> Option<(Vec<u8>, String)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let rest = text.strip_prefix(r#"{"app":""#)?;
    let (app, rest) = rest.split_once(r#"","register_params":""#)?;
    let params = core_types::hex::decode(rest.strip_suffix(r#""}"#)?)?;
    (site_pointer(&params, app)? == bytes).then(|| (params, app.to_string()))
}

/// An app's WEB part: `files` as a deterministic tar in stored-chunk xz -- what the node unpacks and serves, and
/// what a SITE carries under its record (builder#117). `app_container` frames exactly this.
pub fn app_web(files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    if files.is_empty() {
        return Err("an app container with no files".into());
    }
    Ok(xz_stored(&tar(files)?))
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// CRC-32 (IEEE 802.3, reflected), as xz's headers and CRC32 check use it.
fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// `webapp`'s params for a state: its blake3.
pub fn params(state: &[u8]) -> [u8; 32] {
    *blake3::hash(state).as_bytes()
}

/// The address the node serves `state` under (`/v1/contract/web/<address>/`),
/// as freenet-stdlib derives it from the `webapp` code and the params.
pub fn address(webapp_code: &[u8], state: &[u8]) -> String {
    ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(webapp_code.to_vec())),
        Parameters::from(params(state).to_vec()),
    )))
    .key()
    .id()
    .encode()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read back what `tar` wrote: (path, size, checksum-ok) per entry.
    fn entries(t: &[u8]) -> Vec<(String, usize, bool)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + BLOCK <= t.len() && t[at..at + BLOCK].iter().any(|&b| b != 0) {
            let h = &t[at..at + BLOCK];
            let name = String::from_utf8_lossy(&h[..100]).trim_end_matches('\0').to_string();
            let size = usize::from_str_radix(std::str::from_utf8(&h[124..135]).unwrap(), 8).unwrap();
            let stored = u32::from_str_radix(std::str::from_utf8(&h[148..154]).unwrap(), 8).unwrap();
            let mut blank = h.to_vec();
            blank[148..156].copy_from_slice(b"        ");
            let sum: u32 = blank.iter().map(|&b| b as u32).sum();
            out.push((name, size, sum == stored));
            at += BLOCK + size.div_ceil(BLOCK) * BLOCK;
        }
        out
    }

    #[test]
    fn the_same_files_in_any_order_make_the_same_bytes() {
        let a = tar(&[("b.wasm", b"bbb"), ("a.wasm", &[7u8; 700])]).unwrap();
        let b = tar(&[("a.wasm", &[7u8; 700]), ("b.wasm", b"bbb")]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len() % BLOCK, 0);
        assert_eq!(
            entries(&a),
            vec![("a.wasm".into(), 700, true), ("b.wasm".into(), 3, true)],
            "the archive does not read back as written, with valid checksums"
        );
        // THE CONTROL: different content, different bytes.
        assert_ne!(a, tar(&[("a.wasm", &[8u8; 700]), ("b.wasm", b"bbb")]).unwrap());
    }

    #[test]
    fn a_path_a_node_would_refuse_to_unpack_is_refused_here() {
        for bad in ["../x", "/etc/x", "", &"p".repeat(101)] {
            assert!(tar(&[(bad, b"x")]).is_err(), "{bad:?} was archived");
        }
    }

    #[test]
    fn the_framing_is_the_nodes_and_the_address_is_stdlibs() {
        let s = container(b"{}", b"web");
        assert_eq!(&s[..8], &2u64.to_be_bytes());
        assert_eq!(&s[10..18], &3u64.to_be_bytes());
        assert_eq!(s.len(), 8 + 2 + 8 + 3);
        let a = address(b"code", &s);
        assert_eq!(a, address(b"code", &s), "the address is not a function of its inputs");
        assert_ne!(a, address(b"other code", &s));
        assert_ne!(a, address(b"code", &container(b"{}", b"wEb")));
    }
}

#[cfg(test)]
mod pointer {
    use super::*;

    #[test]
    fn a_pointer_is_its_app_and_register_params_and_nothing_else() {
        assert_eq!(site_pointer(&[0xab, 0x01], "notes").unwrap(), br#"{"app":"notes","register_params":"ab01"}"#.to_vec());
        assert_eq!(site_pointer(&[0xab, 0x01], "notes"), site_pointer(&[0xab, 0x01], "notes"), "not byte-identical twice");
        assert!(site_pointer(&[], "notes").is_none() && site_pointer(&[1], "No\"tes").is_none() && site_pointer(&[1], "").is_none());
    }
}

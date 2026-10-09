//! The APP container's xz (builder#104), checked by the REAL `xz`: a stream
//! this crate wrote must be one the node's liblzma reads, and our own reader
//! agreeing with our own writer would prove nothing.

use std::io::Write;
use std::process::{Command, Stdio};
use wire::webapp::{app_container, tar, xz_stored};

fn real_xz(args: &[&str], input: &[u8]) -> (bool, Vec<u8>, String) {
    let mut c = Command::new("xz").args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("xz is installed (the SDK build pins it)");
    let mut stdin = c.stdin.take().expect("stdin");
    let feed = input.to_vec();
    let t = std::thread::spawn(move || stdin.write_all(&feed));
    let o = c.wait_with_output().expect("xz ran");
    t.join().expect("fed").expect("written");
    (o.status.success(), o.stdout, String::from_utf8_lossy(&o.stderr).into())
}

/// Sizes around every boundary the writer has: empty, one byte, a chunk
/// exactly, a chunk plus one, several chunks, and index/block padding cases.
#[test]
fn the_real_xz_decodes_what_this_writes_byte_for_byte() {
    for n in [0usize, 1, 3, 4, 5, 65_535, 65_536, 65_537, 3 * 65_536 + 7, 700_001] {
        let data: Vec<u8> = (0..n).map(|i| (i * 31 % 251) as u8).collect();
        let xz = xz_stored(&data);
        let (ok, out, err) = real_xz(&["-dc", "--single-stream"], &xz);
        assert!(ok, "xz refused {n} bytes: {err}");
        assert_eq!(out.len(), n, "xz decoded a different length for {n} bytes");
        assert!(out == data, "xz decoded different bytes for {n} bytes");
        let (ok, _, err) = real_xz(&["-t"], &xz);
        assert!(ok, "xz -t found {n} bytes' stream corrupt: {err}");
        assert_eq!(xz, xz_stored(&data), "not deterministic at {n} bytes");
    }
}

/// THE CONTROL: the real xz refuses a stream this writes once one byte of it
/// is wrong — so "xz decoded it" is a check that can fail.
#[test]
fn control_one_flipped_byte_is_refused_by_the_real_xz() {
    let data = b"index.html and a loader".repeat(100);
    let good = xz_stored(&data);
    for at in [7, 14, 20, good.len() / 2, good.len() - 20, good.len() - 3] {
        let mut bad = good.clone();
        bad[at] ^= 0x40;
        let (ok, out, _) = real_xz(&["-dc"], &bad);
        assert!(!ok || out != data, "a flipped byte at {at} still decoded to the same data");
    }
}

/// An app container: the node's framing around the stored xz of the sorted
/// tar — unpacked by the real xz and tar, it is the files.
#[test]
fn an_app_container_unpacks_to_its_files_with_the_real_tools() {
    let files: [(&str, &[u8]); 3] = [("index.html", b"<!doctype html>"), ("app.json", b"{}"), ("js/loader.js", b"export {}")];
    let c = app_container(&files).expect("packs");
    assert_eq!(&c[..8], &0u64.to_be_bytes(), "metadata is not empty");
    let web_len = u64::from_be_bytes(c[8..16].try_into().expect("8")) as usize;
    assert_eq!(c.len(), 16 + web_len, "the framing does not add up");
    let (ok, t, err) = real_xz(&["-dc"], &c[16..]);
    assert!(ok, "{err}");
    assert_eq!(t, tar(&files).expect("tar"));
    let dir = std::env::temp_dir().join(format!("webapp-xz-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let mut x = Command::new("tar").args(["-xf", "-", "-C"]).arg(&dir).stdin(Stdio::piped()).spawn().expect("tar");
    x.stdin.take().expect("stdin").write_all(&t).expect("fed");
    assert!(x.wait().expect("tar ran").success());
    for (p, body) in files {
        assert_eq!(std::fs::read(dir.join(p)).expect("unpacked"), body, "{p}");
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(app_container(&[]).is_err(), "an empty app container was packed");
}

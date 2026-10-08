//! MEASURE (TOKEN §5a): sequential blake3 — each iteration hashes the last 32-byte output, one core.
use std::time::Instant;
fn main() {
    let n: u64 = std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(20_000_000);
    let mut x = [7u8; 32];
    let t = Instant::now();
    for _ in 0..n {
        x = *blake3::hash(&x).as_bytes();
    }
    let s = t.elapsed().as_secs_f64();
    println!("{n} iterations in {s:.3} s = {:.0} per second (last {:02x}{:02x})", n as f64 / s, x[0], x[1]);
}

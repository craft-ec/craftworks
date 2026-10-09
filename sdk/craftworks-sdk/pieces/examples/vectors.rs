//! Print the FROZEN load-piece vectors ([`pieces::vectors`]) to stdout.
//! Usage: cargo run -p pieces --example vectors > tests/js/pieces-vectors.json
fn main() {
    print!("{}", pieces::vectors::json());
}

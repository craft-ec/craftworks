//! COPY A FEED (a tail contract) from one node to another: its state as the FROM node holds it, PUT whole to the TO
//! node (the contract checks its own signatures — a copy cannot be forged). `ids` only: each writer's feed id printed.
//!   copy_tail <tail.wasm> <table> ids <writer hex>...
//!   copy_tail <tail.wasm> <table> copy <from ws> <to ws> <writer hex>
use anyhow::{bail, Result};

fn key(h: &str) -> Result<[u8; 32]> {
    let b = hex::decode(h)?;
    b.try_into().map_err(|_| anyhow::anyhow!("32 bytes"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let code = std::fs::read(&a[1])?;
    let table = &a[2];
    match a[3].as_str() {
        "ids" => {
            for w in &a[4..] {
                let o = craftworks_data::Open::new(&code, &key(w)?, table);
                println!("{} {}", &w[..8], hex::encode(o.id_bytes()));
            }
        }
        "copy" => {
            let (from, to) = (&a[4], &a[5]);
            let o = craftworks_data::Open::new(&code, &key(&a[6])?, table);
            let id = o.id_bytes();
            let Some(state) = node_io::get(from, id).await? else { bail!("the FROM node does not hold {}", hex::encode(id)) };
            println!("got {} bytes from the FROM node for {}", state.len(), hex::encode(id));
            let (said, ms) = node_io::put(to, node_io::container(&code, o.params.clone()), state).await?;
            println!("PUT to the TO node: {said} in {ms} ms");
        }
        x => bail!("unknown command {x}"),
    }
    Ok(())
}

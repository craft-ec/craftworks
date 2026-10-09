//! LIVE: realnet step 9's TWO publications on ONE page -- a data commit (the head) and the site republish whose floor
//! names it -- reach another node: `probe::site_probe` with data. usage: PAGE_PORT=<base> PAGE_TMP=<dir>
//! live-site-head <signer.wasm> <block.wasm> <register.wasm> <site.wasm>

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    probe::site_probe::run_both("live-site-head", true).await
}

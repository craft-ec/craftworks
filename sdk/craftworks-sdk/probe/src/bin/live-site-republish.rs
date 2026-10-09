//! LIVE: a SITE REPUBLISHED FROM THE SAME PAGE reaches another node -- `probe::site_probe` with no data commit (the
//! site's own write path). usage: PAGE_PORT=<base> PAGE_TMP=<dir> live-site-republish <signer.wasm> <block.wasm>
//! <register.wasm> <site.wasm>

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    probe::site_probe::run_both("live-site-republish", false).await
}

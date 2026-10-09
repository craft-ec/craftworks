//! A page's `PageIo` for a GIVEN signing key, provisioning its node's signer:
//! what the page does with a key it minted (`Session::mint_if_needed`), with
//! the key handed in. ONE construction for every live tool that drives the
//! page path with a key of its own (live-page-writes, provision-signer).
use page::server::{Server, SignerFacts};
use page::{Ms, Page, PutPath};
use page_io::{Artefacts, PageIo};

/// The Register is named as the page names a minted key's
/// (`wire::register_params(pubkey, HEAD_NAME)`), and `PageIo::provision`
/// registers the signer delegate and sends its `Provision`: the page's own
/// path, never a second encoder.
pub fn for_key(sk: &ed25519_dalek::SigningKey, signer_wasm: &[u8], block_code: Vec<u8>, register_code: Vec<u8>) -> PageIo {
    let (container, signer) = wire::delegate_from_code(signer_wasm);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code, register_code, register_params: wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME), signer },
    );
    io.provision(container, sk.to_bytes().to_vec());
    io
}

//! A SITE: a published app's one stable address (builder#117). A site is a Register record under the SAME
//! authority as the person's head, with the label `site:<app>`: params `RG01 ‖ 0 ‖ key ‖ "site:" ‖ app`, a record
//! `(seq = version, value = blake3(web))`. The site contract (freenet-contracts site/) holds it as its web
//! framing's metadata. Mode 0 only, as `register::head_state` is.

use craftec_register_contract::wire::{Authority, Params};

/// A site label's prefix: `site:` then the app id.
pub const LABEL_PREFIX: &[u8] = b"site:";
/// THE ONE relabelling (builder#117): the same authority, another label. `None` when `params` are not a mode-0
/// Register's -- a k-of-n keyset is Phase 6's, refused rather than guessed.
/// The params are parsed by the Register crate; the label is their TAIL, so the same authority under another label is
/// the params minus their label, plus the new one -- no RG01 layout written here (sdk#364).
pub fn relabel(params: &[u8], label: &[u8]) -> Option<Vec<u8>> {
    let p = Params::parse(params)?;
    if !matches!(p.authority, Authority::One(_)) {
        return None;
    }
    let mut out = params[..params.len() - p.label.len()].to_vec();
    out.extend_from_slice(label);
    Params::parse(&out).is_some().then_some(out)
}

/// `app`'s site params, from the person's Register params: `None` when `app` is not an app id
/// (`core_types::name::app_ok`, the one rule) or the params are not mode 0.
pub fn site_params(register_params: &[u8], app: &str) -> Option<Vec<u8>> {
    if !core_types::name::app_ok(app) {
        return None;
    }
    let mut label = LABEL_PREFIX.to_vec();
    label.extend_from_slice(app.as_bytes());
    relabel(register_params, &label)
}

/// The observation tree's head label (OBSERVABILITY §1, sdk#399): its ONE owner; `wire::OBS_NAME` is this constant.
pub const OBS_LABEL: &[u8] = b"obs";

/// The person's observation-tree head params, from their Register params: the same authority, the label `obs`.
/// `None` when the params are not mode 0.
pub fn obs_params(register_params: &[u8]) -> Option<Vec<u8>> {
    relabel(register_params, OBS_LABEL)
}

/// The node's web framing `[meta length u64 BE][meta][web length u64 BE][web]`, parsed exactly: `(meta, web)`.
pub fn framing(state: &[u8]) -> Option<(&[u8], &[u8])> {
    let (m, rest) = state.split_at_checked(8)?;
    let m = usize::try_from(u64::from_be_bytes(m.try_into().ok()?)).ok()?;
    let (meta, rest) = rest.split_at_checked(m)?;
    let (w, rest) = rest.split_at_checked(8)?;
    let w = usize::try_from(u64::from_be_bytes(w.try_into().ok()?)).ok()?;
    (rest.len() == w).then_some((meta, rest))
}

/// The node's web framing around a site record: what a site PUT carries.
pub fn frame(meta: &[u8], web: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + meta.len() + web.len());
    out.extend_from_slice(&(meta.len() as u64).to_be_bytes());
    out.extend_from_slice(meta);
    out.extend_from_slice(&(web.len() as u64).to_be_bytes());
    out.extend_from_slice(web);
    out
}

//! THE IDENTITY DELEGATE the node runs: the identity's rules (`craftworks_identity::serve_bytes`) over the real delegate
//! context, and UPKEEP when the node wakes it. The entry is behind `freenet-main-delegate` (OFF natively).

pub mod recode;
pub mod rekey;
pub mod rekey_round;
pub mod table;
pub mod write;
pub mod upkeep;
#[cfg(test)]
mod table_tests;
#[cfg(test)]
mod tests;

// A delegate has no randomness of its own: upkeep's pool (stirred by the page, ratcheted per draw: `identity::
// upkeep_random`) ARMS a stream here before MLS draws; nothing is drawn unarmed.
static mut DRAW: Option<[u8; 32]> = None;
pub fn arm_random(seed: [u8; 32]) {
    unsafe { DRAW = Some(seed) };
}
#[cfg(all(target_arch = "wasm32", feature = "freenet-main-delegate"))]
fn upkeep_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    let Some(seed) = (unsafe { DRAW }) else { return Err(getrandom::Error::UNSUPPORTED) };
    let mut out = blake3::Hasher::new_derive_key("craftworks identity upkeep stream");
    out.update(&seed);
    out.finalize_xof().fill(buf);
    unsafe { DRAW = Some(blake3::derive_key("craftworks identity upkeep stream next", &seed)) };
    Ok(())
}
#[cfg(all(target_arch = "wasm32", feature = "freenet-main-delegate"))]
getrandom::register_custom_getrandom!(upkeep_getrandom);

// Nor a clock: upkeep's (the page's time, a minute per wake-up since), set before MLS reads it.
#[cfg(feature = "freenet-main-delegate")]
static mut NOW_S: u64 = 0;
#[cfg(all(target_arch = "wasm32", feature = "freenet-main-delegate"))]
fn clock() -> u64 {
    unsafe { NOW_S }
}

#[cfg(feature = "freenet-main-delegate")]
mod entry {
    use craftworks_identity as identity;
    use freenet_stdlib::prelude::*;

    struct Ctx<'a>(&'a mut DelegateCtx);

    impl identity::Host for Ctx<'_> {
        fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
            self.0.get_secret(key)
        }
        fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
            self.0.set_secret(key, value)
        }
    }

    /// Upkeep's clock, in ms, set for MLS too (in seconds).
    fn clock_now(c: &Ctx) -> u64 {
        let s = identity::upkeep_now(c).unwrap_or(0);
        unsafe { crate::NOW_S = s };
        #[cfg(target_arch = "wasm32")]
        mls_rs_core::time::set_clock(crate::clock);
        s * 1000
    }

    /// Upkeep's asks, as the node's messages.
    fn send(c: &Ctx, io: Vec<crate::upkeep::Io>) -> Vec<OutboundDelegateMsg> {
        use crate::upkeep::{Code, Io};
        let mut out = Vec::new();
        for x in io {
            match x {
                Io::Get { id, subscribe } => {
                    let id = ContractInstanceId::new(id);
                    if subscribe {
                        out.push(OutboundDelegateMsg::SubscribeContractRequest(SubscribeContractRequest::new(id)));
                    }
                    out.push(OutboundDelegateMsg::GetContractRequest(GetContractRequest::new(id)));
                }
                Io::Put { code, params, state } => {
                    let key = match code {
                        Code::Bag => identity::UPKEEP_BAG,
                        Code::Tail => identity::UPKEEP_TAIL,
                        Code::Sealed => identity::UPKEEP_SEALED,
                        Code::Piece => identity::UPKEEP_PIECE,
                    };
                    let Some(code) = identity::Host::get_secret(c, key) else { continue };
                    let contract = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
                        std::sync::Arc::new(ContractCode::from(code)),
                        Parameters::from(params),
                    )));
                    out.push(OutboundDelegateMsg::PutContractRequest(PutContractRequest::new(contract, WrappedState::new(state), RelatedContracts::default())));
                }
                Io::Update { id, delta } => out.push(OutboundDelegateMsg::UpdateContractRequest(UpdateContractRequest::new(
                    ContractInstanceId::new(id),
                    UpdateData::Delta(StateDelta::from(delta)),
                ))),
            }
        }
        out
    }

    fn reply(answer: Vec<u8>) -> OutboundDelegateMsg {
        OutboundDelegateMsg::ApplicationMessage(ApplicationMessage::new(answer).processed(true))
    }

    pub struct Identity;

    // THE MANIFEST: woken every minute (node ≥ 0.2.139, once the person grants this app Background) for UPKEEP with no
    // page open; a node that predates wake-ups ignores `wakeups` and runs it as before.
    #[delegate(manifest(lifecycle = [NodeStarted], capabilities = [Background], wakeups = [upkeep = 60]))]
    impl DelegateInterface for Identity {
        fn process(
            ctx: &mut DelegateCtx,
            _params: Parameters<'static>,
            origin: Option<MessageOrigin>,
            inbound: InboundDelegateMsg,
        ) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
            match inbound {
                InboundDelegateMsg::ApplicationMessage(m) => {
                    // THE GATE: only a web app the node names is served; `None` (the node cannot say who asks) and
                    // another delegate are refused before any request is read.
                    let app = match origin {
                        Some(MessageOrigin::WebApp(id)) => Some(*id),
                        _ => None,
                    };
                    match identity::serve_bytes(&mut Ctx(ctx), &m.payload, app) {
                        identity::Out::Answer(answer) => Ok(vec![reply(answer)]),
                        // A question only the person can answer: the NODE shows it (naming the asking app itself), and
                        // re-enters here with their choice.
                        identity::Out::Ask(p) => Ok(vec![OutboundDelegateMsg::RequestUserInput(UserInputRequest {
                            request_id: p.id,
                            message: NotificationMessage::try_from(&serde_json::Value::String(p.message))
                                .map_err(|_| DelegateError::Other("prompt".into()))?,
                            responses: p.choices.into_iter().map(|c| ClientResponse::new(c.into_bytes())).collect(),
                        })]),
                    }
                }
                InboundDelegateMsg::UserResponse(r) => Ok(identity::serve_answer(&mut Ctx(ctx), r.request_id, &r.response)
                    .map(reply)
                    .into_iter()
                    .collect()),
                // A WAKE-UP: counted; the account's inbox read (and watched, so the node keeps it current); an
                // upkeep round started (admitting askers) when no page runs.
                InboundDelegateMsg::WakeupFired { .. } => {
                    let mut c = Ctx(ctx);
                    let mut out = match identity::upkeep_woke(&mut c) {
                        Some(inbox) => {
                            let id = ContractInstanceId::new(inbox);
                            vec![
                                OutboundDelegateMsg::SubscribeContractRequest(SubscribeContractRequest::new(id)),
                                OutboundDelegateMsg::GetContractRequest(GetContractRequest::new(id)),
                            ]
                        }
                        None => Vec::new(),
                    };
                    let now = clock_now(&c);
                    let io = crate::upkeep::woke(&mut c, now);
                    out.extend(send(&c, io));
                    // RE-KEYS (a space's files after a removal), side by side with admissions.
                    let io = crate::rekey_round::woke(&mut c);
                    out.extend(send(&c, io));
                    Ok(out)
                }
                // The inbox as the node holds it; anything else, an answer upkeep waits for.
                InboundDelegateMsg::GetContractResponse(r) => {
                    let mut c = Ctx(ctx);
                    let id: [u8; 32] = *r.contract_id;
                    if identity::upkeep_inbox(&c) == Some(id) {
                        identity::upkeep_read(&mut c, r.state.as_ref().map(|s| s.as_ref().len() as u64).unwrap_or(0));
                    }
                    let now = clock_now(&c);
                    let io = crate::upkeep::replied(&mut c, crate::upkeep::Reply::Got { id, state: r.state.map(|s| s.as_ref().to_vec()) }, now);
                    Ok(send(&c, io))
                }
                InboundDelegateMsg::PutContractResponse(r) => {
                    let mut c = Ctx(ctx);
                    let now = clock_now(&c);
                    let io = crate::upkeep::replied(&mut c, crate::upkeep::Reply::Put { id: *r.contract_id, ok: r.result.is_ok() }, now);
                    Ok(send(&c, io))
                }
                InboundDelegateMsg::UpdateContractResponse(r) => {
                    let mut c = Ctx(ctx);
                    let now = clock_now(&c);
                    let io = crate::upkeep::replied(&mut c, crate::upkeep::Reply::Updated { id: *r.contract_id, ok: r.result.is_ok() }, now);
                    Ok(send(&c, io))
                }
                InboundDelegateMsg::Lifecycle(_) => Ok(Vec::new()),
                _ => Ok(Vec::new()),
            }
        }
    }
}

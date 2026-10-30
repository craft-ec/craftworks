//! THE IDENTITY DELEGATE the node runs: the identity's rules (`craftworks_identity::serve_bytes`) over the real delegate
//! context, and UPKEEP when the node wakes it. The entry is behind `freenet-main-delegate` (OFF natively).

pub mod file_io;
pub mod recode;
pub mod rekey;
pub mod rekey_round;
pub mod table;
pub mod write;
pub mod upkeep;
pub mod reads;
pub mod witness_round;
pub mod witness_statement;
pub mod keeper_round;
pub mod account_round;
pub mod codes;
#[cfg(test)]
mod table_tests;
#[cfg(test)]
mod follow_tests;
#[cfg(test)]
mod testhost;
#[cfg(test)]
mod witness_tests;
#[cfg(test)]
mod statement_tests;
#[cfg(test)]
mod keeper_tests;
#[cfg(test)]
mod account_tests;

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

    // THE SECRET STORE's OWN ANSWER (10-28, fx/fy: "instance mem space not recorded" — the node lost this instance's
    // memory, and every secret call failed ~20 000 times a second): `DelegateCtx` turns ANY failure into "absent", so a
    // run read nothing, wrote nothing, and went on as if nothing were held. Only NOT FOUND is absent here; any other
    // failure stops the call (a trap: the node answers it with an error, the next call runs afresh) — nothing is
    // decided, scanned or written from a store that did not answer.
    #[cfg(target_family = "wasm")]
    #[link(wasm_import_module = "freenet_delegate_secrets")]
    extern "C" {
        fn __frnt__delegate__get_secret_len(key_ptr: i64, key_len: i32) -> i32;
        fn __frnt__delegate__get_secret(key_ptr: i64, key_len: i32, out_ptr: i64, out_len: i32) -> i32;
    }

    impl identity::Host for Ctx<'_> {
        #[cfg(target_family = "wasm")]
        fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
            use freenet_stdlib::delegate_host::error_codes::ERR_SECRET_NOT_FOUND;
            let len = unsafe { __frnt__delegate__get_secret_len(key.as_ptr() as i64, key.len() as i32) };
            if len == ERR_SECRET_NOT_FOUND {
                return None;
            }
            assert!(len >= 0, "the secret store did not answer (code {len})");
            let mut out = vec![0u8; len as usize];
            if len > 0 {
                let n = unsafe { __frnt__delegate__get_secret(key.as_ptr() as i64, key.len() as i32, out.as_mut_ptr() as i64, out.len() as i32) };
                assert!(n >= 0, "the secret store did not answer (code {n})");
                out.truncate(n as usize);
            }
            Some(out)
        }
        #[cfg(not(target_family = "wasm"))]
        fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
            self.0.get_secret(key)
        }
        fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
            assert!(self.0.set_secret(key, value), "the secret store did not keep a secret");
            true
        }
    }

    /// Upkeep's clock, in ms, set for MLS too (in seconds).
    fn clock_now(c: &Ctx) -> u64 {
        let s = identity::upkeep_now(c).unwrap_or(0);
        unsafe { crate::NOW_S = s };
        #[cfg(target_arch = "wasm32")]
        mls_rs_core::time::set_clock(crate::clock);
        s.saturating_mul(1000)
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
                    let Some(code) = crate::codes::get(c, key) else { continue };
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

    /// THE NODE's DEBUG TRACE at the one door: what came in, and every request this call sends out (`identity::upkeep_trace`).
    fn traced(c: &mut Ctx, inbound: String, io: &[crate::upkeep::Io]) {
        use crate::upkeep::Io;
        let h8 = |b: &[u8; 32]| b[..4].iter().map(|x| format!("{x:02x}")).collect::<String>();
        let mut lines = vec![format!("in {inbound}")];
        lines.extend(io.iter().map(|x| match x {
            Io::Get { id, subscribe } => format!("out GET {}{}", h8(id), if *subscribe { " +sub" } else { "" }),
            Io::Put { code, params, state } => format!("out PUT {code:?} params {}B state {}B", params.len(), state.len()),
            Io::Update { id, delta } => format!("out UPDATE {} {}B", h8(id), delta.len()),
        }));
        identity::upkeep_trace(c, &lines);
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
                    let mut c = Ctx(ctx);
                    match identity::serve_bytes(&mut c, &m.payload, app) {
                        identity::Out::Answer(answer) => {
                            let mut out = vec![reply(answer)];
                            // THE FOLLOWER NOW (a page asked): its round begun at once, when none is running.
                            if let Some(member) = identity::upkeep_take_now(&mut c) {
                                let now = clock_now(&c);
                                let io = crate::upkeep::now(&mut c, member, now);
                                traced(&mut c, "a page asked: the round now".into(), &io);
                                out.extend(send(&c, io));
                            }
                            Ok(out)
                        }
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
                    let mut all = crate::upkeep::woke(&mut c, now);
                    // RE-KEYS (a space's files after a removal), side by side with admissions.
                    all.extend(crate::rekey_round::woke(&mut c));
                    // WITNESSING (TOKEN §7a), by the member's mandate, with no page open.
                    all.extend(crate::witness_round::woke(&mut c));
                    // KEEPING (TOKEN §7a): the beacon's challenges answered, by the member's mandate, with no page open.
                    all.extend(crate::keeper_round::woke(&mut c));
                    // THE ACCOUNT's TREE (K8.5): a device added or taken out, with no page of the account open.
                    all.extend(crate::account_round::woke(&mut c));
                    traced(&mut c, "wake-up".into(), &all);
                    out.extend(send(&c, all));
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
                    let reply = crate::upkeep::Reply::Got { id, state: r.state.map(|s| s.as_ref().to_vec()) };
                    let mut io = crate::witness_round::replied(&mut c, &reply);
                    io.extend(crate::keeper_round::replied(&mut c, &reply));
                    io.extend(crate::account_round::replied(&mut c, &reply));
                    let got = match &reply { crate::upkeep::Reply::Got { state: Some(st), .. } => format!("{}B", st.len()), _ => "NOT FOUND".into() };
                    io.extend(crate::upkeep::replied(&mut c, reply, now));
                    io.extend(crate::rekey_round::run_now(&mut c));
                    traced(&mut c, format!("GOT {} {got}", id[..4].iter().map(|x| format!("{x:02x}")).collect::<String>()), &io);
                    Ok(send(&c, io))
                }
                InboundDelegateMsg::PutContractResponse(r) => {
                    let mut c = Ctx(ctx);
                    let now = clock_now(&c);
                    let (id, ok): ([u8; 32], bool) = (*r.contract_id, r.result.is_ok());
                    let mut io = crate::upkeep::replied(&mut c, crate::upkeep::Reply::Put { id, ok }, now);
                    io.extend(crate::rekey_round::run_now(&mut c));
                    traced(&mut c, format!("PUT {} {}", id[..4].iter().map(|x| format!("{x:02x}")).collect::<String>(), if ok { "ok" } else { "REFUSED" }), &io);
                    Ok(send(&c, io))
                }
                InboundDelegateMsg::UpdateContractResponse(r) => {
                    let mut c = Ctx(ctx);
                    let now = clock_now(&c);
                    let (id, ok): ([u8; 32], bool) = (*r.contract_id, r.result.is_ok());
                    let mut io = crate::upkeep::replied(&mut c, crate::upkeep::Reply::Updated { id, ok }, now);
                    io.extend(crate::rekey_round::run_now(&mut c));
                    traced(&mut c, format!("UPDATED {} {}", id[..4].iter().map(|x| format!("{x:02x}")).collect::<String>(), if ok { "ok" } else { "REFUSED" }), &io);
                    Ok(send(&c, io))
                }
                // A SUBSCRIPTION's answer and a SUBSCRIBED contract's change: traced (10-30 — measuring whether the node
                // tells a delegate of a change at all, before the rounds are driven by it).
                InboundDelegateMsg::SubscribeContractResponse(r) => {
                    let mut c = Ctx(ctx);
                    let id: [u8; 32] = *r.contract_id;
                    traced(&mut c, format!("SUBSCRIBED {} {}", id[..4].iter().map(|x| format!("{x:02x}")).collect::<String>(), match &r.result { Ok(()) => "ok".to_string(), Err(e) => format!("REFUSED {e}") }), &[]);
                    Ok(Vec::new())
                }
                InboundDelegateMsg::ContractNotification(n) => {
                    let mut c = Ctx(ctx);
                    let id: [u8; 32] = *n.contract_id;
                    // (A change: the follower's pass now — `upkeep::notified`.)
                    let io = crate::upkeep::notified(&mut c);
                    traced(&mut c, format!("NOTIFIED {} {}B", id[..4].iter().map(|x| format!("{x:02x}")).collect::<String>(), n.new_state.as_ref().len()), &io);
                    Ok(send(&c, io))
                }
                InboundDelegateMsg::Lifecycle(_) => Ok(Vec::new()),
                _ => Ok(Vec::new()),
            }
        }
    }
}

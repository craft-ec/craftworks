//! The entry the node calls: `serve_bytes` over the real delegate context. Behind `freenet-main-delegate` (OFF
//! natively).

use freenet_stdlib::prelude::*;

struct Ctx<'a>(&'a mut DelegateCtx);

impl crate::Host for Ctx<'_> {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get_secret(key)
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        self.0.set_secret(key, value)
    }
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
                match crate::serve_bytes(&mut Ctx(ctx), &m.payload, app) {
                    crate::Out::Answer(answer) => Ok(vec![reply(answer)]),
                    // A question only the person can answer: the NODE shows it (naming the asking app itself), and
                    // re-enters here with their choice.
                    crate::Out::Ask(p) => Ok(vec![OutboundDelegateMsg::RequestUserInput(UserInputRequest {
                        request_id: p.id,
                        message: NotificationMessage::try_from(&serde_json::Value::String(p.message))
                            .map_err(|_| DelegateError::Other("prompt".into()))?,
                        responses: p.choices.into_iter().map(|c| ClientResponse::new(c.into_bytes())).collect(),
                    })]),
                }
            }
            InboundDelegateMsg::UserResponse(r) => Ok(crate::serve_answer(&mut Ctx(ctx), r.request_id, &r.response)
                .map(reply)
                .into_iter()
                .collect()),
            // A WAKE-UP: counted; the account's inbox read (and watched, so the node keeps it current).
            InboundDelegateMsg::WakeupFired { .. } => {
                let mut c = Ctx(ctx);
                Ok(match crate::upkeep_woke(&mut c) {
                    Some(inbox) => {
                        let id = ContractInstanceId::new(inbox);
                        vec![
                            OutboundDelegateMsg::SubscribeContractRequest(SubscribeContractRequest::new(id)),
                            OutboundDelegateMsg::GetContractRequest(GetContractRequest::new(id)),
                        ]
                    }
                    None => Vec::new(),
                })
            }
            // The inbox as the node holds it.
            InboundDelegateMsg::GetContractResponse(r) => {
                crate::upkeep_read(&mut Ctx(ctx), r.state.map(|s| s.as_ref().len() as u64).unwrap_or(0));
                Ok(Vec::new())
            }
            InboundDelegateMsg::Lifecycle(_) => Ok(Vec::new()),
            // The identity issues no GET, PUT, UPDATE or SUBSCRIBE, so nothing else can answer it.
            _ => Ok(Vec::new()),
        }
    }
}

// A delegate has no randomness of its own: upkeep's pool, stirred by the page, ratcheted per draw (never before a stir).
static mut DRAW: Option<[u8; 32]> = None;
fn upkeep_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    let Some(seed) = (unsafe { DRAW }) else { return Err(getrandom::Error::UNSUPPORTED) };
    let mut out = blake3::Hasher::new_derive_key("craftworks identity upkeep stream");
    out.update(&seed);
    out.finalize_xof().fill(buf);
    unsafe { DRAW = Some(blake3::derive_key("craftworks identity upkeep stream next", &seed)) };
    Ok(())
}
getrandom::register_custom_getrandom!(upkeep_getrandom);

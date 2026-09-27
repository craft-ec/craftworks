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

#[delegate]
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
            // The identity issues no GET, PUT, UPDATE or SUBSCRIBE, so nothing else can answer it.
            _ => Ok(Vec::new()),
        }
    }
}

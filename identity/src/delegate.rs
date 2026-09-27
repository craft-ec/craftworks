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
                let who = match origin {
                    None => crate::Origin::Local,
                    Some(MessageOrigin::WebApp(id)) => crate::Origin::App(*id),
                    Some(_) => crate::Origin::Delegate,
                };
                let answer = crate::serve_bytes(&mut Ctx(ctx), &m.payload, who);
                Ok(vec![OutboundDelegateMsg::ApplicationMessage(ApplicationMessage::new(answer).processed(true))])
            }
            // The identity issues no GET, PUT, UPDATE or SUBSCRIBE, so nothing else can answer it.
            _ => Ok(Vec::new()),
        }
    }
}

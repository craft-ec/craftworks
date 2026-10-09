//! The entry the node calls: `serve_full` over the real delegate context. Behind `freenet-main-delegate` (OFF
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
    fn contract_state(&self, instance_id: &[u8; 32]) -> Option<Vec<u8>> {
        self.0.get_contract_state(instance_id)
    }
}

fn message(bytes: Vec<u8>) -> OutboundDelegateMsg {
    OutboundDelegateMsg::ApplicationMessage(ApplicationMessage::new(bytes).processed(true))
}

pub struct Signer;

#[delegate]
impl DelegateInterface for Signer {
    fn process(
        ctx: &mut DelegateCtx,
        _params: Parameters<'static>,
        origin: Option<MessageOrigin>,
        inbound: InboundDelegateMsg,
    ) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
        match inbound {
            InboundDelegateMsg::ApplicationMessage(m) => {
                // WHO ASKED, as the node attests it (builder#117): nothing attested is the person's own tools;
                // a served web app or a delegate (possibly relaying one) is `Served`, and signs no site.
                let who = if origin.is_some() { crate::Origin::Served } else { crate::Origin::Local };
                let served = crate::serve_full(&mut Ctx(ctx), &m.payload, who);
                Ok(vec![message(crate::reply(&served))])
            }
            // The signer issues no GET, PUT, UPDATE or SUBSCRIBE, so nothing else can answer it.
            _ => Ok(Vec::new()),
        }
    }
}

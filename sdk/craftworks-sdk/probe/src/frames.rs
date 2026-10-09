//! THE ONE DECODER of client→node frames for the probes: a frame is a freenet-stdlib `ClientRequest` (bincode,
//! the client API's native encoding), and a request the sender split (`StreamChunk`) is reassembled, per socket,
//! with stdlib's own `ReassemblyBuffer`. classify-frames and ws-withhold both read requests through this, so the
//! rule for "what did the page send" lives once.
use freenet_stdlib::client_api::streaming::ReassemblyBuffer;
use freenet_stdlib::client_api::ClientRequest;
use std::collections::BTreeMap;

/// Whole requests out of a stream of frames, reassembling chunked ones per socket.
#[derive(Default)]
pub struct Requests {
    streams: BTreeMap<String, ReassemblyBuffer>,
}

/// One frame's outcome.
pub enum Frame {
    /// A whole request (a plain frame, or the last chunk of a split one).
    Whole(ClientRequest<'static>),
    /// A chunk of a request still being reassembled.
    Partial,
}

impl Requests {
    /// Read one frame sent on `socket`. An error names why the bytes are not a request.
    pub fn push(&mut self, socket: &str, bytes: &[u8]) -> Result<Frame, String> {
        let req: ClientRequest<'_> = bincode::deserialize(bytes).map_err(|e| format!("not a ClientRequest: {e}"))?;
        if let ClientRequest::StreamChunk { stream_id, index, total, data } = &req {
            let buf = self.streams.entry(socket.to_string()).or_default();
            return match buf.receive_chunk(*stream_id, *index, *total, data.clone()) {
                Ok(None) => Ok(Frame::Partial),
                Ok(Some(whole)) => bincode::deserialize::<ClientRequest<'_>>(&whole)
                    .map(|r| Frame::Whole(r.into_owned()))
                    .map_err(|e| format!("a reassembled stream is not a ClientRequest: {e}")),
                Err(e) => Err(format!("a stream chunk did not reassemble: {e:?}")),
            };
        }
        Ok(Frame::Whole(req.into_owned()))
    }
}

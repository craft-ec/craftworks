//! What a LIVE page probe (a `live-*` binary on a private node) needs every time: its clock, its socket, and one wait
//! on that socket. One home, so a probe adds only what it measures.

use anyhow::{bail, Result};
use futures::{SinkExt, StreamExt};
use page::Ms;
use page_io::PageIo;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

/// A probe's socket to its node.
pub type Sock = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The page's clock: an epoch-like origin, never 0 (sdk#397: a zero-origin clock hid an RTO defect).
pub fn now_ms(t0: Instant) -> u64 {
    1_000 + t0.elapsed().as_millis() as u64
}

/// ONE wait on the socket, no longer than `io`'s next due time (capped at 50 ms): a binary frame's bytes, or `None`
/// -- `io` ticked on a timeout, or a frame that is not binary. A socket error or close is an error.
pub async fn next_frame(sock: &mut Sock, io: &mut PageIo, t0: Instant) -> Result<Option<Vec<u8>>> {
    let wait = io.next_due().map_or(50, |d| d.0.saturating_sub(now_ms(t0)).clamp(1, 50));
    match tokio::time::timeout(Duration::from_millis(wait), sock.next()).await {
        Ok(Some(Ok(Message::Binary(b)))) => Ok(Some(b.to_vec())),
        Ok(Some(Ok(_))) => Ok(None),
        Ok(Some(Err(e))) => bail!("the socket: {e}"),
        Ok(None) => bail!("the node closed the socket"),
        Err(_) => {
            io.tick(Ms(now_ms(t0)));
            Ok(None)
        }
    }
}

/// One turn of a live probe's loop: every frame the page has, sent; every reply it has, decoded onto `replies`.
pub async fn flush(sock: &mut Sock, io: &mut PageIo, replies: &mut Vec<protocol::Reply>) -> Result<()> {
    for f in io.take_frames() {
        sock.send(Message::Binary(f.into())).await.map_err(|e| anyhow::anyhow!("send: {e}"))?;
    }
    for r in io.take_replies() {
        replies.push(protocol::decode_reply(&r).map_err(|d| anyhow::anyhow!("a reply that does not decode: {d:?}"))?);
    }
    Ok(())
}

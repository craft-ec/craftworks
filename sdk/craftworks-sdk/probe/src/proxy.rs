//! ONE PAGE <-> NODE PROXY on loopback, for the probes that stand between a page and its node (`ws-withhold`,
//! `ws-lose`): a connection that is not a WebSocket upgrade (the node's HTTP -- web containers, pieces) is piped through
//! unchanged; a WebSocket is bridged frame by frame through the binary's [`Hooks`]. The node is dialled only when a
//! client connects. Both ports go through the probes' one guard against the owner's nodes ([`crate::node::allowed_port`]).
use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;

/// What a proxy does with the WebSocket frames it bridges. `conn` numbers the page's connections, from 0.
pub trait Hooks: Send + Sync + 'static {
    /// A frame from the page, on its way to the node unchanged.
    fn up(&self, _conn: u64, _m: &Message) {}
    /// What goes on to the node for a frame from the page, in order, and what the page is answered INSTEAD (the node
    /// never sees a request it is not handed). Default: the frame itself, and nothing else (ws-drop's is the one other).
    fn pass_up(&self, _conn: u64, m: Message) -> (Vec<Message>, Option<Message>) {
        (vec![m], None)
    }
    /// A frame from the node: what the page is sent instead (`None`: nothing).
    fn down(&self, conn: u64, m: Message) -> Option<Message>;
}

/// Where a probe's EVIDENCE lines go (one JSON value per line): stderr by default -- the harness reads them there --
/// or a test's capture, so a test sees what the hooks really say, not what their formatters would.
pub struct Log(Box<dyn Fn(&serde_json::Value) + Send + Sync>);

impl Default for Log {
    fn default() -> Log {
        Log(Box::new(|l| eprintln!("{l}")))
    }
}

impl Log {
    pub fn to(f: impl Fn(&serde_json::Value) + Send + Sync + 'static) -> Log {
        Log(Box::new(f))
    }

    pub fn say(&self, line: &serde_json::Value) {
        (self.0)(line)
    }
}

/// Is this the first packet of a WebSocket upgrade? (Peeked, not consumed.)
async fn is_upgrade(s: &TcpStream) -> bool {
    let mut buf = [0u8; 2048];
    match s.peek(&mut buf).await {
        Ok(n) => String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase().contains("upgrade: websocket"),
        Err(_) => false,
    }
}

async fn pipe(client: TcpStream, node_port: u16) -> Result<()> {
    let node = TcpStream::connect(("127.0.0.1", node_port)).await.context("the node")?;
    let (mut cr, mut cw) = client.into_split();
    let (mut nr, mut nw) = node.into_split();
    let a = tokio::io::copy(&mut cr, &mut nw);
    let b = tokio::io::copy(&mut nr, &mut cw);
    let _ = tokio::join!(a, b);
    Ok(())
}

// The handshake callback's `Result<Response, ErrorResponse>` is tungstenite's signature, not ours to shrink.
#[allow(clippy::result_large_err)]
async fn websocket(client: TcpStream, node_port: u16, conn: u64, hooks: Arc<dyn Hooks>) -> Result<()> {
    let mut path = String::new();
    let client = tokio_tungstenite::accept_hdr_async(client, |req: &Request, resp: Response| {
        path = req.uri().to_string();
        Ok(resp)
    })
    .await
    .context("accepting the page's WebSocket")?;
    let (node, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{node_port}{path}")).await.context("the node's WebSocket")?;
    let (mut c_tx, mut c_rx) = client.split();
    let (mut n_tx, mut n_rx) = node.split();
    let up_hooks = hooks.clone();
    // A hook's own answer to the page (a request it did not hand the node) goes down beside the node's answers.
    let (to_page, mut from_hooks) = tokio::sync::mpsc::unbounded_channel::<Message>();
    let up = async move {
        while let Some(m) = c_rx.next().await {
            let m = m?;
            up_hooks.up(conn, &m);
            let (on, instead) = up_hooks.pass_up(conn, m);
            for m in on {
                n_tx.send(m).await?;
            }
            if let Some(a) = instead {
                let _ = to_page.send(a);
            }
        }
        anyhow::Ok(())
    };
    let down = async move {
        loop {
            tokio::select! {
                m = n_rx.next() => {
                    let Some(m) = m else { break };
                    if let Some(m) = hooks.down(conn, m?) {
                        c_tx.send(m).await?;
                    }
                }
                Some(a) = from_hooks.recv() => c_tx.send(a).await?,
            }
        }
        anyhow::Ok(())
    };
    let _ = tokio::join!(up, down);
    Ok(())
}

/// Listen on 127.0.0.1:`listen` and bridge every client to the node on 127.0.0.1:`node`, for ever. `said` is printed
/// (JSON, on stderr) once listening, beside the two ports.
pub async fn serve(listen: u16, node: u16, said: serde_json::Value, hooks: Arc<dyn Hooks>) -> Result<()> {
    let listen = crate::node::allowed_port(&format!("ws://127.0.0.1:{listen}"))?;
    let node = crate::node::allowed_port(&format!("ws://127.0.0.1:{node}"))?;
    let l = TcpListener::bind(("127.0.0.1", listen)).await.with_context(|| format!("binding {listen}"))?;
    let mut line = serde_json::json!({ "listening": listen, "node": node });
    if let (Some(o), serde_json::Value::Object(extra)) = (line.as_object_mut(), said) {
        o.extend(extra);
    }
    eprintln!("{line}");
    let conns = AtomicU64::new(0);
    loop {
        let (s, _) = l.accept().await?;
        let conn = conns.fetch_add(1, Ordering::Relaxed);
        let hooks = hooks.clone();
        tokio::spawn(async move {
            let r = if is_upgrade(&s).await { websocket(s, node, conn, hooks).await } else { pipe(s, node).await };
            if let Err(e) = r {
                eprintln!("{}", serde_json::json!({ "connection": conn, "ended": e.to_string() }));
            }
        });
    }
}

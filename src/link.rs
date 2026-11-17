//! Client side of the add-on protocol: newline-delimited JSON over a Unix socket.
//!
//! Connections are lazy: nothing is opened until the first call, so the MCP server starts fine when Blender is
//! not running. Requests are serialised (the add-on does not support concurrency in v1).

use crate::error::LinkError;
use crate::paths;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_LINE: usize = 16 * 1024 * 1024;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    pub v: u64,
    pub blender: String,
    pub addon: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub headless: bool,
}

struct Conn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    hello: Hello,
}

pub struct Link {
    path: PathBuf,
    conn: Mutex<Option<Conn>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl Link {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            conn: Mutex::new(None),
            next_id: 1.into(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn id(&self) -> u64 {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    async fn connect(&self) -> Result<Conn, LinkError> {
        paths::check_len(&self.path).map_err(LinkError::BadPath)?;
        if let Some(why) = paths::untrusted_reason(&self.path) {
            return Err(LinkError::BadPath(format!("refusing to connect: {why}")));
        }
        let stream =
            match tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(&self.path)).await {
                Err(_) => return Err(LinkError::Timeout(CONNECT_TIMEOUT)),
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    return Err(match e.kind() {
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                            LinkError::NotRunning
                        }
                        _ => LinkError::Io(e.to_string()),
                    });
                }
            };
        let (r, w) = stream.into_split();
        let mut conn = Conn {
            reader: BufReader::new(r),
            writer: w,
            hello: Hello {
                v: 0,
                blender: String::new(),
                addon: String::new(),
                features: vec![],
                headless: false,
            },
        };
        let id = self.id();
        let result = exchange(
            &mut conn,
            id,
            "hello",
            json!({"client": "omnix-blender", "v": PROTOCOL_VERSION}),
            None,
            CONNECT_TIMEOUT,
        )
        .await
        .map_err(|(e, _)| e)?;
        let hello: Hello = serde_json::from_value(result)
            .map_err(|e| LinkError::Protocol(format!("bad hello: {e}")))?;
        if hello.v != u64::from(PROTOCOL_VERSION) {
            return Err(LinkError::VersionMismatch {
                ours: PROTOCOL_VERSION,
                theirs: hello.v,
            });
        }
        conn.hello = hello;
        Ok(conn)
    }

    /// Connect (if needed) and return the add-on's hello.
    pub async fn hello(&self) -> Result<Hello, LinkError> {
        let mut guard = self.conn.lock().await;
        if guard.is_none() {
            *guard = Some(self.connect().await?);
        }
        Ok(guard.as_ref().expect("just set").hello.clone())
    }

    /// Call `method`. `safe_retry` allows one transparent retry on a fresh connection when a reused connection
    /// turned out to be dead before any response byte arrived (only safe for read-only methods).
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        safe_retry: bool,
    ) -> Result<Value, LinkError> {
        // Requests are serialised; do not wait longer than the caller's own budget for our turn.
        let mut guard = tokio::time::timeout(timeout, self.conn.lock())
            .await
            .map_err(|_| LinkError::Timeout(timeout))?;
        // Drop a connection the peer already closed (Blender restarted) so we reconnect instead of failing.
        if let Some(c) = guard.as_mut()
            && let Ok(Ok(buf)) = tokio::time::timeout(Duration::ZERO, c.reader.fill_buf()).await
            && buf.is_empty()
        {
            *guard = None;
        }
        for attempt in 0..2 {
            let reused = guard.is_some();
            if guard.is_none() {
                *guard = Some(self.connect().await?);
            }
            let id = self.id();
            let conn = guard.as_mut().expect("just set");
            // The add-on needs a little extra time over the client budget to report its own timeout.
            let server_timeout = timeout.as_secs_f64();
            match exchange(
                conn,
                id,
                method,
                params.clone(),
                Some(server_timeout),
                timeout + Duration::from_secs(2),
            )
            .await
            {
                Ok(v) => return Ok(v),
                Err((e, got_bytes)) => {
                    let dead = matches!(e, LinkError::Io(_) | LinkError::Protocol(_));
                    if dead {
                        *guard = None;
                    }
                    let wrote_nothing_useful = !got_bytes;
                    if attempt == 0 && reused && dead && wrote_nothing_useful && safe_retry {
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(LinkError::NotRunning)
    }
}

/// Send one request and wait for the response with the matching id. Returns the error plus whether any response
/// byte was received (to decide on retry).
async fn exchange(
    conn: &mut Conn,
    id: u64,
    method: &str,
    params: Value,
    timeout_s: Option<f64>,
    budget: Duration,
) -> Result<Value, (LinkError, bool)> {
    let mut req = json!({"v": PROTOCOL_VERSION, "id": id, "method": method, "params": params});
    if let Some(t) = timeout_s {
        req["timeout_s"] = json!(t);
    }
    let mut line =
        serde_json::to_vec(&req).map_err(|e| (LinkError::Protocol(e.to_string()), false))?;
    if line.len() > MAX_LINE {
        return Err((LinkError::Protocol("request exceeds 16 MiB".into()), false));
    }
    line.push(b'\n');
    let mut got_bytes = false;
    let fut = async {
        conn.writer
            .write_all(&line)
            .await
            .map_err(|e| LinkError::Io(e.to_string()))?;
        loop {
            let mut buf = Vec::new();
            let n = (&mut conn.reader)
                .take(MAX_LINE as u64 + 1)
                .read_until(b'\n', &mut buf)
                .await
                .map_err(|e| LinkError::Io(e.to_string()))?;
            if n == 0 {
                return Err(LinkError::Io("connection closed by Blender".into()));
            }
            got_bytes = true;
            if buf.len() > MAX_LINE || buf.last() != Some(&b'\n') {
                return Err(LinkError::Protocol(
                    "response line too long or truncated".into(),
                ));
            }
            let msg: Value = serde_json::from_slice(&buf)
                .map_err(|e| LinkError::Protocol(format!("bad JSON from add-on: {e}")))?;
            // Skip late answers to earlier (timed out) requests.
            match msg.get("id") {
                Some(Value::Number(n)) if n.as_u64() == Some(id) => {}
                Some(Value::Null) | None => {}
                _ => continue,
            }
            if msg.get("ok") == Some(&Value::Bool(true)) {
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
            let e = msg.get("error").cloned().unwrap_or(Value::Null);
            return Err(LinkError::Remote {
                code: e
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("blender_error")
                    .to_string(),
                message: e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string(),
            });
        }
    };
    let res = tokio::time::timeout(budget, fut).await;
    match res {
        Err(_) => Err((LinkError::Timeout(budget), got_bytes)),
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err((e, got_bytes)),
    }
}

// ported from moxxy-ai/moxxy crates/moxxy-runtime/src/browser/sidecar.rs
//
// Owns a `tokio::process::Child` running the Playwright sidecar.
// Multiplexes concurrent JSON-RPC calls over a single stdin pipe
// (one writer at a time, behind a Mutex) and routes responses back
// to per-request oneshot channels via a background stdout pump.
//
// The error type is codeless-native (ToolError). Moxxy's
// PrimitiveError categories collapse into Failed/InvalidArgs at the
// boundary — the granularity moxxy needed for its agent loop doesn't
// match what codeless tasks consume.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

use super::config::BrowserManagerConfig;
use super::protocol::{RpcError, RpcResponse};
use crate::error::ToolError;

/// JSON line cap. Matches moxxy: large enough for a full screenshot
/// returned base64-encoded plus JSON overhead.
const READ_LINE_CAP: usize = 64 * 1024 * 1024;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<RpcResponse>>>>;

pub struct SidecarProcess {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: Pending,
    /// Set once the reader observes EOF or a decode failure. Subsequent
    /// `request` calls fail fast rather than hanging on a dead pipe.
    dead: Arc<AtomicBool>,
}

impl SidecarProcess {
    pub async fn spawn(config: &BrowserManagerConfig) -> Result<Self, ToolError> {
        tracing::info!(
            node = %config.node_bin.display(),
            script = %config.sidecar_script.display(),
            "spawning playwright sidecar",
        );

        let mut cmd = Command::new(&config.node_bin);
        for arg in &config.node_args {
            cmd.arg(arg);
        }
        cmd.arg(&config.sidecar_script)
            .env("PLAYWRIGHT_BROWSERS_PATH", &config.browsers_dir)
            .env("NODE_NO_WARNINGS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::failed(format!("spawn sidecar: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ToolError::failed("sidecar stdin not piped".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::failed("sidecar stdout not piped".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::failed("sidecar stderr not piped".to_string()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let dead = Arc::new(AtomicBool::new(false));

        spawn_stderr_pump(stderr);
        spawn_stdout_pump(stdout, pending.clone(), dead.clone());

        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            next_id: AtomicU64::new(1),
            pending,
            dead,
        })
    }

    /// Send a JSON-RPC request and await its response under a hard
    /// timeout. The caller's cancellation token (when wired in by
    /// higher-level tools) selects against this future to enable
    /// per-call cancellation.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, ToolError> {
        if self.dead.load(Ordering::SeqCst) {
            return Err(ToolError::failed("sidecar is not running".to_string()));
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = serde_json::json!({ "id": id, "method": method, "params": params });
        let line = serde_json::to_string(&req)
            .map_err(|e| ToolError::failed(format!("encode request: {e}")))?;

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        if let Err(e) = self.write_request(&line).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => decode_response(resp),
            Ok(Err(_)) => Err(ToolError::failed(
                "sidecar response channel dropped".to_string(),
            )),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(ToolError::failed(format!(
                    "sidecar request timed out after {timeout:?}"
                )))
            }
        }
    }

    async fn write_request(&self, line: &str) -> Result<(), ToolError> {
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ToolError::failed(format!("write to sidecar: {e}")))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| ToolError::failed(format!("write newline: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| ToolError::failed(format!("flush sidecar stdin: {e}")))?;
        Ok(())
    }

    /// Send `shutdown` RPC, then SIGKILL as a backstop. Idempotent.
    pub async fn shutdown(&self) {
        let _ = self
            .request("shutdown", serde_json::json!({}), Duration::from_secs(3))
            .await;
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
    }

    pub fn is_alive(&self) -> bool {
        !self.dead.load(Ordering::SeqCst)
    }
}

impl Drop for SidecarProcess {
    fn drop(&mut self) {
        // tokio::process::Child's kill_on_drop(true) handles the OS
        // side; flipping `dead` here ensures any racing request call
        // sees the process as gone immediately.
        self.dead.store(true, Ordering::SeqCst);
    }
}

fn decode_response(resp: RpcResponse) -> Result<serde_json::Value, ToolError> {
    if resp.ok {
        Ok(resp.result.unwrap_or(serde_json::Value::Null))
    } else {
        let err = resp.error.unwrap_or(RpcError {
            code: "unknown".into(),
            message: "sidecar returned ok=false with no error".into(),
        });
        Err(rpc_to_tool_error(err))
    }
}

fn rpc_to_tool_error(err: RpcError) -> ToolError {
    match err.code.as_str() {
        "invalid_params" => ToolError::invalid_args(err.message),
        // Moxxy distinguishes timeout / size_limit / not_found at the
        // primitive layer. Codeless tasks consume strings; we flatten
        // and prefix the code so callers can still pattern-match on
        // the message if they care.
        other => ToolError::failed(format!("{other}: {}", err.message)),
    }
}

fn spawn_stderr_pump(stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim_end();
                    if !trimmed.is_empty() {
                        tracing::info!(target: "playwright_sidecar", "{trimmed}");
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn spawn_stdout_pump(stdout: tokio::process::ChildStdout, pending: Pending, dead: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let mut reader = BufReader::with_capacity(64 * 1024, stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match read_capped_line(&mut reader, &mut line, READ_LINE_CAP).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "sidecar stdout read error");
                    break;
                }
            }
            let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
            if trimmed.is_empty() {
                continue;
            }
            let resp: RpcResponse = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "sidecar response decode failed");
                    continue;
                }
            };
            if let Some(id) = resp.id {
                let mut map = pending.lock().await;
                if let Some(tx) = map.remove(&id) {
                    let _ = tx.send(resp);
                }
            }
        }
        dead.store(true, Ordering::SeqCst);
        // Fail every still-pending request so they don't hang.
        let mut map = pending.lock().await;
        for (_, tx) in map.drain() {
            let _ = tx.send(RpcResponse {
                id: None,
                ok: false,
                result: None,
                error: Some(RpcError {
                    code: "sidecar_dead".into(),
                    message: "sidecar process exited".into(),
                }),
            });
        }
        tracing::info!("sidecar reader task exited");
    });
}

/// Like `BufReader::read_line` but caps the number of bytes read per
/// line. Without the cap a malformed sidecar could allocate
/// indefinitely on a missing newline.
async fn read_capped_line<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
    buf: &mut String,
    cap: usize,
) -> std::io::Result<usize> {
    let start = buf.len();
    loop {
        let (consumed, done) = {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                return Ok(buf.len() - start);
            }
            if let Some(idx) = available.iter().position(|b| *b == b'\n') {
                let take = &available[..=idx];
                buf.push_str(
                    std::str::from_utf8(take)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
                );
                (idx + 1, true)
            } else {
                buf.push_str(
                    std::str::from_utf8(available)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
                );
                (available.len(), false)
            }
        };
        reader.consume(consumed);
        if done {
            return Ok(buf.len() - start);
        }
        if buf.len() - start > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "sidecar response exceeded line length cap",
            ));
        }
    }
}

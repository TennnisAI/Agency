//! Drive a web-served agent's own HTTP API after `dsh web` (or a peer) binds.
//!
//! The CLI has no flag for a working folder or an opening prompt — `dsh web`'s
//! whole family is `--host`, `--port`, `--trusted-host`, `--no-open`. Both are
//! RPC: `workspace.create({ path })` adopts the directory the process was
//! launched in, and `session.prompt` is the opening ask a terminal agent would
//! have taken on argv. Read off `@deepseek-ai/dsh-host-apiproxy` 0.1.1-rc.2 and
//! the headless recipe in discussion #2707 (workspace.create over RPC).
//!
//! Observed 2026-08-29: their `/api` responses are `Transfer-Encoding: chunked`
//! with no `Content-Length`. A client that only reads by Content-Length gets an
//! empty body, burns the whole wait budget on "EOF while parsing JSON", never
//! adopts the folder, and never delivers the prompt — while the iframe stays on
//! "Starting the GUI…" the entire time.
//!
//! The iframe must not load until the workspace exists: their
//! `startInitialSelection` runs once, and an empty list marks the pass done
//! without ever picking a folder. `gui_live` waits on [`ReadySet`].

use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Host plugin source: collapses the conversation sidebar once on first paint.
/// Written under the Agency data dir and pointed at by [`ensure_collapse_sidebar_patch`].
const COLLAPSE_SIDEBAR_PLUGIN: &str = include_str!("dsh_agency/collapse_sidebar_plugin.js");

/// Materialize Agency's dsh `--patch` overlay (collapse sidebar on boot) under
/// `data_dir/dsh-agency/` and return the absolute path of the patch YAML.
///
/// dsh's layout store is transient and always starts with the sidebar open;
/// there is no CLI flag or setting for a collapsed default. The overlay inserts
/// a tiny host plugin that injects a one-shot click of the Collapse control.
/// The patch path must be absolute (dsh resolves plugin `name` that way), and
/// `--patch` is a launcher flag on `dsh web`, so callers place it right after
/// `web` in argv.
pub fn ensure_collapse_sidebar_patch(data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join("dsh-agency");
    fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let plugin = dir.join("collapse-sidebar.js");
    fs::write(&plugin, COLLAPSE_SIDEBAR_PLUGIN)
        .with_context(|| format!("write {}", plugin.display()))?;
    // Prefer a canonical path so spaces/symlinks in the data dir don't break
    // the YAML string dsh's Loader hands to Node's module resolver.
    let plugin_abs = plugin.canonicalize().unwrap_or_else(|_| plugin.clone());
    let plugin_yaml = plugin_abs.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    let patch = dir.join("collapse-sidebar.patch.yml");
    // Build the YAML without `\ ` line continuation: that form strips the next
    // line's leading whitespace, which flattened the list indent and made dsh
    // reject the overlay ("end of the stream or a document separator is expected").
    let yaml = format!(
        "{comment}\n- insert:\n    - id: agency-collapse-sidebar\n      name: \"{plugin_yaml}\"\n",
        comment = "# Written by Agency. Collapses the dsh conversation sidebar on boot.",
        plugin_yaml = plugin_yaml,
    );
    fs::write(&patch, yaml).with_context(|| format!("write {}", patch.display()))?;
    Ok(patch.canonicalize().unwrap_or(patch))
}

/// How long we poll for `/api` after the TCP port is bound. Their Loader tree
/// settles after the listen; with a working body reader this is usually one or
/// two attempts, not a long blank wait.
const API_WAIT: Duration = Duration::from_secs(8);
const API_POLL: Duration = Duration::from_millis(100);
/// After the workspace exists we flip `gui_live` so the iframe loads. Their
/// client then creates (or reuses) a blank session on that workspace. We poll
/// for that session and prompt *it* — prompting a session we minted ourselves
/// before the UI attached often left the ask on a session the user never saw.
const SESSION_WAIT: Duration = Duration::from_secs(8);
const SESSION_POLL: Duration = Duration::from_millis(200);
/// `session.prompt` refuses with `model-unavailable` until a provider key is
/// in place. Retry briefly so an issue dispatched while the user pastes a key
/// still lands; do not block the GUI on this.
const PROMPT_RETRY: Duration = Duration::from_secs(15);

/// Session ids whose GUI handshake has finished (workspace adopted, or the
/// budget expired). `gui_live` stays false until the id is in here, so the
/// iframe does not win the race against `workspace.create`.
pub type ReadySet = Arc<Mutex<HashSet<String>>>;

/// One unary call on the agent's `/api` JSON-RPC carrier.
pub trait Rpc {
    fn call(&self, method: &str, payload: Value) -> Result<Value>;
}

/// Loopback JSON-RPC client for a `dsh web` (or peer) server. The Host fence
/// accepts an IP-literal `Host` on any port; we never send `Origin`.
pub struct HttpRpc {
    port: u16,
}

impl HttpRpc {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}

impl Rpc for HttpRpc {
    fn call(&self, method: &str, payload: Value) -> Result<Value> {
        let rpc_id = uuid::Uuid::new_v4().to_string();
        let body = json!({
            "type": "client-request",
            "rpcId": rpc_id,
            "method": method,
            "payload": payload,
        })
        .to_string();
        let path = format!("/api/{method}");
        let raw = http_post_json(self.port, &path, &body)
            .with_context(|| format!("POST {path} on 127.0.0.1:{}", self.port))?;
        rpc_value(&raw)
    }
}

/// Adopt `cwd` as the GUI's workspace. Idempotent: `workspace.create` returns
/// the existing row for a path it already knows. Does not create a session —
/// their client does that on first paint, and that is the session we must
/// prompt.
pub fn adopt_workspace(rpc: &dyn Rpc, cwd: &Path) -> Result<Adopted> {
    let path = cwd.to_string_lossy().into_owned();
    let created = rpc.call("workspace.create", json!({ "path": path }))?;
    let workspace = created
        .get("workspace")
        .ok_or_else(|| anyhow!("workspace.create returned no workspace"))?;
    let workspace_id = workspace
        .get("workspaceId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("workspace.create returned no workspaceId"))?
        .to_string();
    Ok(Adopted { workspace_id })
}

/// The workspace row the GUI will open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopted {
    pub workspace_id: String,
}

/// Queue `prompt` onto `session_id`. `mode: queue` is a new turn on a blank
/// session; `steer` is the in-flight interrupt and would be refused here.
pub fn queue_prompt(rpc: &dyn Rpc, session_id: &str, prompt: &str) -> Result<()> {
    rpc.call(
        "session.prompt",
        json!({
            "sessionId": session_id,
            "mode": "queue",
            "content": [{ "type": "text", "text": prompt }],
        }),
    )?;
    Ok(())
}

/// Whether a `session.prompt` failure is the missing-key case we should retry.
pub fn is_model_unavailable(err: &anyhow::Error) -> bool {
    err.to_string().contains("model-unavailable")
}

/// Wait for the API, adopt the workspace, then (if `prompt` is non-empty) find
/// the blank session their UI opens and queue the ask there.
///
/// `on_ready` fires once the workspace exists — that is the `gui_live` flip —
/// even if the later prompt fails. A timeout still calls `on_ready` so the
/// iframe is not stuck on "Starting the GUI" when the server never speaks RPC.
pub fn handshake(port: u16, cwd: &Path, prompt: &str, on_ready: impl FnOnce()) {
    let rpc = HttpRpc::new(port);
    let adopted = match wait_and_adopt(&rpc, cwd) {
        Ok(a) => {
            on_ready();
            a
        }
        Err(e) => {
            log::warn!("web GUI handshake on port {port} failed: {e:#}");
            on_ready();
            return;
        }
    };
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return;
    }
    let session_id = match wait_for_blank_session(&rpc, &adopted.workspace_id) {
        Ok(id) => id,
        Err(e) => {
            log::warn!(
                "web GUI prompt: no blank session on workspace {}: {e:#}",
                adopted.workspace_id
            );
            return;
        }
    };
    let deadline = Instant::now() + PROMPT_RETRY;
    loop {
        match queue_prompt(&rpc, &session_id, prompt) {
            Ok(()) => {
                log::info!("web GUI prompt delivered on session {session_id}");
                return;
            }
            Err(e) if is_model_unavailable(&e) && Instant::now() < deadline => {
                log::info!("web GUI prompt waiting on a model credential (retrying): {e}");
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(e) => {
                log::warn!("web GUI prompt on session {session_id} failed: {e:#}");
                return;
            }
        }
    }
}

/// Spawn the handshake on a thread and record `session_id` in `ready` when the
/// workspace is adopted (or the budget expires). No-op when `port` is None.
pub fn kick(ready: &ReadySet, session_id: &str, port: Option<u16>, cwd: &Path, prompt: &str) {
    let Some(port) = port else { return };
    ready.lock().unwrap().remove(session_id);
    let ready = Arc::clone(ready);
    let id = session_id.to_string();
    let cwd = cwd.to_path_buf();
    let prompt = prompt.to_string();
    std::thread::spawn(move || {
        handshake(port, &cwd, &prompt, || {
            ready.lock().unwrap().insert(id.clone());
        });
    });
}

fn wait_and_adopt(rpc: &HttpRpc, cwd: &Path) -> Result<Adopted> {
    let deadline = Instant::now() + API_WAIT;
    loop {
        match adopt_workspace(rpc, cwd) {
            Ok(a) => return Ok(a),
            Err(e) if Instant::now() < deadline => {
                log::debug!("web GUI API not ready yet: {e}");
                std::thread::sleep(API_POLL);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Prefer a blank session already attached to the workspace (what their UI
/// opens). If none appears, create one ourselves as a fallback.
fn wait_for_blank_session(rpc: &HttpRpc, workspace_id: &str) -> Result<String> {
    let deadline = Instant::now() + SESSION_WAIT;
    loop {
        if let Ok(listed) = rpc.call("session.list", json!({})) {
            if let Ok(ws) = rpc.call("workspace.list", json!({})) {
                if let Some(id) = blank_on_workspace(&listed, &ws, workspace_id) {
                    return Ok(id);
                }
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(SESSION_POLL);
    }
    // Their UI never attached a blank session in time (slow load, or the
    // onboarding modal blocked selection). Mint one on the workspace so the
    // ask still has somewhere to land; the next reload will see it.
    let session = rpc.call("session.create", json!({ "workspaceId": workspace_id }))?;
    session
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("session.create returned no sessionId"))
}

fn blank_on_workspace(listed: &Value, workspaces: &Value, workspace_id: &str) -> Option<String> {
    let attached: Vec<String> = workspaces
        .get("items")?
        .as_array()?
        .iter()
        .find(|w| w.get("workspaceId").and_then(Value::as_str) == Some(workspace_id))?
        .get("sessionIds")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let items = listed.get("items")?.as_array()?;
    for item in items {
        let id = item.get("sessionId")?.as_str()?;
        let blank = item.get("blank").and_then(Value::as_bool).unwrap_or(false);
        if blank && attached.iter().any(|a| a == id) {
            return Some(id.to_string());
        }
    }
    None
}

/// Unwrap a ServerResponse full form into the business value, or name the
/// error code so retries can match `model-unavailable`.
fn rpc_value(raw: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(raw).context("web GUI RPC response was not JSON")?;
    let result = v.get("result").ok_or_else(|| anyhow!("web GUI RPC response had no result"))?;
    if result.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(result.get("value").cloned().unwrap_or(Value::Null));
    }
    let err = result.get("error");
    let code = err.and_then(|e| e.get("code")).and_then(Value::as_str).unwrap_or("unknown");
    let message = err.and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or(code);
    bail!("{code}: {message}")
}

fn http_post_json(port: u16, path: &str, body: &str) -> Result<String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .with_context(|| format!("connecting to 127.0.0.1:{port}"))?;
    // Short: a hung RPC must not pin the iframe on "Starting…" for tens of seconds.
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n\
         {body}",
        body.len()
    );
    stream.write_all(req.as_bytes())?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status)?;
    let status_code = status.split_whitespace().nth(1).unwrap_or("0");
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else { continue };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.to_ascii_lowercase().split(',').any(|t| t.trim() == "chunked");
        }
    }
    // dsh (undici/Node) answers JSON with Transfer-Encoding: chunked and no
    // Content-Length. Reading zero bytes and parsing that as JSON is what made
    // every handshake fail with "EOF while parsing a value" for ~20s.
    let body = if chunked {
        read_chunked_body(&mut reader)?
    } else if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        String::from_utf8_lossy(&buf).into_owned()
    } else {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        String::from_utf8_lossy(&buf).into_owned()
    };
    if status_code != "200" {
        bail!("HTTP {status_code}: {body}");
    }
    Ok(body)
}

/// Decode an HTTP/1.1 chunked body. Pure enough to unit-test the size/line
/// dance without a socket.
fn read_chunked_body<R: BufRead>(reader: &mut R) -> Result<String> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        let size_hex = size_line.trim().split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .with_context(|| format!("invalid chunk size {size_line:?}"))?;
        if size == 0 {
            // Trailing headers + final CRLF. Drain one blank line; ignore extras.
            let mut trailer = String::new();
            let _ = reader.read_line(&mut trailer);
            break;
        }
        let mut chunk = vec![0u8; size];
        reader.read_exact(&mut chunk)?;
        out.extend_from_slice(&chunk);
        // Chunk data is followed by CRLF.
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Cursor;

    struct Script {
        calls: RefCell<Vec<(String, Value)>>,
        responses: RefCell<Vec<Result<Value>>>,
    }

    impl Script {
        fn new(responses: Vec<Result<Value>>) -> Self {
            Self { calls: RefCell::new(Vec::new()), responses: RefCell::new(responses) }
        }
    }

    impl Rpc for Script {
        fn call(&self, method: &str, payload: Value) -> Result<Value> {
            self.calls.borrow_mut().push((method.to_string(), payload));
            let mut q = self.responses.borrow_mut();
            if q.is_empty() {
                bail!("unexpected extra RPC {method}");
            }
            q.remove(0)
        }
    }

    #[test]
    fn adopt_returns_the_workspace_id() {
        let rpc = Script::new(vec![Ok(json!({
            "workspace": {
                "workspaceId": "ws-1",
                "path": "/tmp/wt",
                "sessionIds": [],
            }
        }))]);
        let got = adopt_workspace(&rpc, Path::new("/tmp/wt")).unwrap();
        assert_eq!(got, Adopted { workspace_id: "ws-1".into() });
        let calls = rpc.calls.borrow();
        assert_eq!(calls[0].0, "workspace.create");
        assert_eq!(calls[0].1["path"], "/tmp/wt");
        assert_eq!(calls.len(), 1, "must not mint a session before the UI attaches");
    }

    #[test]
    fn blank_on_workspace_picks_the_attached_blank_session() {
        let listed = json!({
            "items": [
                { "sessionId": "s-old", "blank": false },
                { "sessionId": "s-blank", "blank": true },
                { "sessionId": "s-other", "blank": true },
            ]
        });
        let workspaces = json!({
            "items": [{
                "workspaceId": "ws-1",
                "sessionIds": ["s-old", "s-blank"],
            }]
        });
        assert_eq!(blank_on_workspace(&listed, &workspaces, "ws-1").as_deref(), Some("s-blank"));
        assert_eq!(blank_on_workspace(&listed, &workspaces, "ws-missing"), None);
    }

    #[test]
    fn queue_prompt_is_a_queued_text_turn() {
        let rpc = Script::new(vec![Ok(json!({ "accepted": true }))]);
        queue_prompt(&rpc, "s-1", "work on AGE-1").unwrap();
        let (method, payload) = &rpc.calls.borrow()[0];
        assert_eq!(method, "session.prompt");
        assert_eq!(payload["sessionId"], "s-1");
        assert_eq!(payload["mode"], "queue");
        assert_eq!(payload["content"][0]["type"], "text");
        assert_eq!(payload["content"][0]["text"], "work on AGE-1");
    }

    #[test]
    fn rpc_value_names_the_business_error_code() {
        let raw = r#"{"type":"server-response","rpcId":"x","result":{"ok":false,"error":{"code":"model-unavailable","message":"no route","details":{}}}}"#;
        let err = rpc_value(raw).unwrap_err().to_string();
        assert!(err.starts_with("model-unavailable:"), "{err}");
        assert!(is_model_unavailable(&anyhow!(err)));
    }

    #[test]
    fn rpc_value_unwraps_the_business_value() {
        let raw = r#"{"type":"server-response","rpcId":"x","result":{"ok":true,"value":{"sessionId":"s-1"}}}"#;
        assert_eq!(rpc_value(raw).unwrap()["sessionId"], "s-1");
    }

    /// The failure observed 2026-08-29: dsh answers with chunked JSON and no
    /// Content-Length. Reading zero bytes left every handshake on an empty body.
    #[test]
    fn reads_a_chunked_json_body_like_dsh_sends() {
        let payload = r#"{"type":"server-response","rpcId":"t1","result":{"ok":true,"value":{"workspace":{"workspaceId":"ws-1"}}}}"#;
        let chunked = format!("{:x}\r\n{payload}\r\n0\r\n\r\n", payload.len());
        let got = read_chunked_body(&mut Cursor::new(chunked.into_bytes())).unwrap();
        assert_eq!(got, payload);
        assert_eq!(rpc_value(&got).unwrap()["workspace"]["workspaceId"], "ws-1");
    }

    #[test]
    fn reads_a_chunked_body_split_across_chunks() {
        let chunked = "5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let got = read_chunked_body(&mut Cursor::new(chunked.as_bytes())).unwrap();
        assert_eq!(got, "hello world");
    }

    #[test]
    fn collapse_sidebar_patch_points_at_a_real_plugin_file() {
        let dir = tempfile::tempdir().unwrap();
        let patch = ensure_collapse_sidebar_patch(dir.path()).unwrap();
        assert!(patch.is_absolute(), "{patch:?}");
        let yaml = fs::read_to_string(&patch).unwrap();
        // Nested under `insert:` — a flat `- id:` sibling is what crashed dsh
        // (YAMLException at the `name:` line) on 2026-08-29.
        assert!(
            yaml.contains("\n- insert:\n    - id: agency-collapse-sidebar\n      name: \""),
            "patch must keep list indent, got:\n{yaml}"
        );
        let plugin = dir.path().join("dsh-agency/collapse-sidebar.js");
        assert!(plugin.is_file(), "{plugin:?}");
        assert!(
            fs::read_to_string(&plugin).unwrap().contains("Collapse sidebar"),
            "plugin must target the Collapse control"
        );
        // Absolute plugin path inside the quoted name field.
        assert!(
            yaml.contains(&format!("name: \"{}\"", plugin.canonicalize().unwrap().display())),
            "{yaml}"
        );
    }
}

//! The preview MCP server: Agency serving tools *to* a dispatched agent.
//!
//! [`crate::mcp`] emits MCP configuration *for* each agent; this module is the
//! other direction (AGE-143). Per run whose project has a web run script,
//! Agency listens on the last port of that run's port block — loopback only,
//! never any other interface — and serves two things from one origin:
//!
//! - `/__agency__/…`: a stateless MCP endpoint (tools scoped to this one
//!   run's preview and nothing else), the bridge script, and the bridge's
//!   own hello/poll/result/log endpoints.
//! - everything else: a reverse proxy of the workspace's dev server that
//!   injects the bridge into HTML ([`proxy`]), which is what the Run tab's
//!   preview iframe actually renders.
//!
//! The bridge long-polls for commands, so a tool call becomes: MCP request →
//! command queued → parked poll returns it → the page executes → result posted
//! → MCP response. Console and network logs need no round trip: the bridge
//! streams console entries in, the proxy logs traffic as it forwards, and the
//! read tools drain buffers. All the waiting lives in [`Broker`], one mutex
//! and one condvar; the protocol on either side of it is pure and tested.
//!
//! The agent's config gets the endpoint through the same machinery as every
//! other server — [`server_entry`] rides into [`crate::mcp::emit_for_agent`]'s
//! merged list — so nothing here invents a second emission path.

mod http;
pub(crate) mod proxy;
mod rpc;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::BufReader;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The MCP server name agents see in their config and tool prefixes.
pub const SERVER_NAME: &str = "agency-preview";
/// Endpoint paths live under a prefix no real app is likely to claim; the
/// whole prefix is reserved and never proxied.
pub const MCP_PATH: &str = "/__agency__/mcp";
const CONTROL_PREFIX: &str = "/__agency__/";

/// How long the bridge's poll parks before returning empty.
const POLL_PARK: Duration = Duration::from_secs(20);
/// How long a tool call waits for the page to answer a command.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
/// A bridge whose page neither polls nor reports for this long is gone.
const BRIDGE_STALE: Duration = Duration::from_secs(30);
/// Ring-buffer caps. Old entries drop with a count, never silently.
const LOG_CAP: usize = 400;
/// Concurrent connections beyond this are refused; a page load bursts a few
/// dozen asset requests, so the cap only ever bites a runaway client.
const MAX_CONNS: usize = 64;

/// The preview server's port inside a workspace's port block: the last one, so
/// it can be computed from config alone (stable across app restarts, which
/// keeps the URL already emitted into an agent's MCP config valid). The app
/// port is the block's first; a block too small to hold both gets no preview
/// server rather than a port fight.
pub fn mcp_port(base: u16, block_size: u16) -> Option<u16> {
    if block_size < 2 {
        return None;
    }
    base.checked_add(block_size - 1)
}

/// The MCP config entry that points an agent at a run's preview server, ready
/// to merge into the per-agent emission in [`crate::mcp::emit_for_agent`].
pub fn server_entry(mcp_port: u16) -> crate::mcp::McpServer {
    crate::mcp::McpServer {
        name: SERVER_NAME.to_string(),
        url: Some(format!("http://127.0.0.1:{mcp_port}{MCP_PATH}")),
        transport: Some(crate::mcp::McpTransport::Http),
        ..Default::default()
    }
}

/// Whether anything accepts connections on a loopback port. Cheap enough to
/// ask per poll; used both for the app's "should the hidden preview host
/// mount" question and for choosing the right guidance in tool errors.
pub fn serving(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

/// Facts injected into tool error text at call time, so the guidance an agent
/// reads carries the actual command and never goes stale. Kept out of tool
/// descriptions on purpose: those land in the agent's context and must stay
/// prompt-cache-stable.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// The project's web run script, as (name, command).
    pub script: Option<(String, String)>,
}

/// What the embedding app provides: fresh facts for guidance, and the native
/// pixel screenshot of the preview pane (which only the app, owner of the
/// window, can take — see the app crate's preview_shot).
#[derive(Clone)]
pub struct Hooks {
    pub facts: Arc<dyn Fn() -> Facts + Send + Sync>,
    pub screenshot: Arc<dyn Fn() -> std::result::Result<Vec<u8>, String> + Send + Sync>,
}

/// One console line reported by the bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
}

/// One request the preview made, as the proxy saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkEntry {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub content_type: String,
    pub bytes: u64,
    pub ms: u64,
}

/// An append-only ring with a "since the last read" cursor. Entries evicted
/// past the cap are counted, so a read can say what it missed instead of
/// pretending the quiet buffer was the whole story.
struct Log<T> {
    entries: VecDeque<(u64, T)>,
    next: u64,
    cursor: u64,
}

impl<T: Clone> Log<T> {
    fn new() -> Log<T> {
        Log { entries: VecDeque::new(), next: 0, cursor: 0 }
    }

    fn push(&mut self, item: T) {
        self.entries.push_back((self.next, item));
        self.next += 1;
        while self.entries.len() > LOG_CAP {
            self.entries.pop_front();
        }
    }

    /// Everything since the previous drain, plus how many entries fell off the
    /// buffer unread. Advances the cursor.
    fn drain_new(&mut self) -> (Vec<T>, u64) {
        let first_kept = self.entries.front().map(|(s, _)| *s).unwrap_or(self.next);
        let missed = first_kept.saturating_sub(self.cursor);
        let out = self
            .entries
            .iter()
            .filter(|(s, _)| *s >= self.cursor)
            .map(|(_, t)| t.clone())
            .collect();
        self.cursor = self.next;
        (out, missed)
    }
}

struct Cmd {
    id: u64,
    payload: Value,
}

struct BrokerState {
    console: Log<ConsoleEntry>,
    network: Log<NetworkEntry>,
    /// The bridge instance currently driving the preview. A page load (or the
    /// visible pane replacing the hidden host) says hello and takes over; polls
    /// from any earlier instance are told to retire.
    active: Option<String>,
    last_seen: Option<Instant>,
    url: Option<String>,
    title: Option<String>,
    /// Pollers parked in [`Broker::poll`] right now — the strongest liveness
    /// signal there is: a queued command will be picked up immediately.
    waiting: usize,
    queue: VecDeque<Cmd>,
    results: HashMap<u64, Value>,
    /// Command ids a tool call is still waiting on; results for anything else
    /// are dropped rather than accumulated.
    awaited: HashSet<u64>,
    next_cmd: u64,
}

struct Broker {
    state: Mutex<BrokerState>,
    wake: Condvar,
}

impl Broker {
    fn new() -> Broker {
        Broker {
            state: Mutex::new(BrokerState {
                console: Log::new(),
                network: Log::new(),
                active: None,
                last_seen: None,
                url: None,
                title: None,
                waiting: 0,
                queue: VecDeque::new(),
                results: HashMap::new(),
                awaited: HashSet::new(),
                next_cmd: 1,
            }),
            wake: Condvar::new(),
        }
    }

    fn push_network(&self, entry: NetworkEntry) {
        self.state.lock().unwrap().network.push(entry);
    }

    fn hello(&self, instance: &str, url: Option<String>, title: Option<String>) {
        let mut st = self.state.lock().unwrap();
        st.active = Some(instance.to_string());
        st.last_seen = Some(Instant::now());
        st.url = url;
        st.title = title;
        // Parked polls from a replaced instance wake up and get retired.
        self.wake.notify_all();
    }

    fn log(&self, instance: &str, entries: Vec<ConsoleEntry>) {
        let mut st = self.state.lock().unwrap();
        if st.active.as_deref() != Some(instance) {
            return;
        }
        st.last_seen = Some(Instant::now());
        for e in entries {
            st.console.push(e);
        }
    }

    /// The bridge's long poll: park until a command is queued for this
    /// instance, the instance is replaced, or the park times out.
    fn poll(&self, instance: &str) -> Value {
        let deadline = Instant::now() + POLL_PARK;
        let mut st = self.state.lock().unwrap();
        loop {
            if st.active.as_deref() != Some(instance) {
                return json!({ "retire": true });
            }
            st.last_seen = Some(Instant::now());
            if let Some(cmd) = st.queue.pop_front() {
                return json!({ "id": cmd.id, "cmd": cmd.payload });
            }
            let now = Instant::now();
            if now >= deadline {
                return json!({});
            }
            st.waiting += 1;
            let (guard, _) = self.wake.wait_timeout(st, deadline - now).unwrap();
            st = guard;
            st.waiting -= 1;
        }
    }

    fn result(&self, id: u64, result: Value) {
        let mut st = self.state.lock().unwrap();
        st.last_seen = Some(Instant::now());
        if st.awaited.contains(&id) {
            st.results.insert(id, result);
            self.wake.notify_all();
        }
    }

    fn bridge_alive(st: &BrokerState) -> bool {
        st.active.is_some()
            && (st.waiting > 0 || st.last_seen.is_some_and(|t| t.elapsed() < BRIDGE_STALE))
    }

    /// Send one command to the live bridge and wait for its answer. `Err` is
    /// guidance text for the agent — why there was nothing to talk to, or what
    /// the silence means.
    fn command(&self, ctx: &Ctx, payload: Value) -> std::result::Result<Value, String> {
        let mut st = self.state.lock().unwrap();
        if !Self::bridge_alive(&st) {
            drop(st);
            return Err(guidance(ctx.app_port, &(ctx.hooks.facts)(), serving(ctx.app_port)));
        }
        let id = st.next_cmd;
        st.next_cmd += 1;
        st.queue.push_back(Cmd { id, payload });
        st.awaited.insert(id);
        self.wake.notify_all();
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            if let Some(v) = st.results.remove(&id) {
                st.awaited.remove(&id);
                return Ok(v);
            }
            let now = Instant::now();
            if now >= deadline {
                st.awaited.remove(&id);
                let picked_up = !st.queue.iter().any(|c| c.id == id);
                st.queue.retain(|c| c.id != id);
                return Err(if picked_up {
                    "The preview page picked the command up but never answered; it probably \
                     navigated or reloaded mid-command. Take a preview_snapshot to see where it \
                     is now, then try again."
                        .to_string()
                } else {
                    "The preview page did not pick the command up in time; it may be reloading. \
                     Try again in a moment."
                        .to_string()
                });
            }
            let (guard, _) = self.wake.wait_timeout(st, deadline - now).unwrap();
            st = guard;
        }
    }
}

/// Why a preview tool has nothing to act on, with the way out spelled from
/// live facts: the actual script command and the actual port.
fn guidance(app_port: u16, facts: &Facts, port_serving: bool) -> String {
    if port_serving {
        return format!(
            "Something is serving on port {app_port}, but the preview page has not connected \
             yet. Agency opens the preview automatically within a few seconds of the server \
             coming up; try again shortly."
        );
    }
    match &facts.script {
        Some((name, command)) => format!(
            "The preview has nothing to show: nothing is serving on port {app_port}. Start the \
             project's web run script \"{name}\" (`{command}`) as a background process in this \
             workspace (AGENCY_PORT={app_port} is already exported in your environment), or ask \
             the user to press Start in Agency's Run tab. Give it a moment to boot, then try \
             again."
        ),
        None => format!(
            "The preview has nothing to show: nothing is serving on port {app_port} and this \
             project has no web run script configured. Serve the app on port {app_port} \
             yourself, or ask the user to configure a run script in Agency's Run tab."
        ),
    }
}

fn format_console(entries: &[ConsoleEntry], missed: u64) -> String {
    if entries.is_empty() {
        return "No console output since the last read.".to_string();
    }
    let mut out = String::new();
    if missed > 0 {
        out.push_str(&format!("({missed} earlier entries dropped past the buffer)\n"));
    }
    for e in entries {
        out.push_str(&format!("[{}] {}\n", e.level, e.text));
    }
    out.trim_end().to_string()
}

fn human_bytes(n: u64) -> String {
    if n >= 1_048_576 {
        format!("{:.1} MB", n as f64 / 1_048_576.0)
    } else if n >= 1024 {
        format!("{:.1} kB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn format_network(entries: &[NetworkEntry], missed: u64) -> String {
    if entries.is_empty() {
        return "No requests since the last read.".to_string();
    }
    let mut out = String::new();
    if missed > 0 {
        out.push_str(&format!("({missed} earlier requests dropped past the buffer)\n"));
    }
    for e in entries {
        out.push_str(&format!(
            "{} {} → {}{}, {}, {} ms\n",
            e.method,
            e.path,
            e.status,
            if e.content_type.is_empty() || e.content_type == "-" {
                String::new()
            } else {
                format!(" {}", e.content_type)
            },
            human_bytes(e.bytes),
            e.ms,
        ));
    }
    out.trim_end().to_string()
}

struct Ctx {
    app_port: u16,
    broker: Broker,
    hooks: Hooks,
    shutdown: AtomicBool,
    conns: AtomicUsize,
}

/// One run's preview server: the listener thread plus the shared broker.
/// Stopping (or dropping) closes the listener; in-flight connection threads
/// finish on their own.
pub struct PreviewServer {
    port: u16,
    app_port: u16,
    started: Instant,
    ctx: Arc<Ctx>,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl PreviewServer {
    /// Bind 127.0.0.1:`port` (0 picks an ephemeral port, for tests) and serve
    /// until stopped. Loopback only is not configurable, deliberately.
    pub fn start(port: u16, app_port: u16, hooks: Hooks) -> Result<PreviewServer> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
            .with_context(|| format!("binding the preview server to 127.0.0.1:{port}"))?;
        let bound = listener.local_addr()?.port();
        let ctx = Arc::new(Ctx {
            app_port,
            broker: Broker::new(),
            hooks,
            shutdown: AtomicBool::new(false),
            conns: AtomicUsize::new(0),
        });
        let ctx2 = ctx.clone();
        let accept =
            std::thread::Builder::new().name(format!("preview-{bound}")).spawn(move || {
                for stream in listener.incoming() {
                    if ctx2.shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    if ctx2.conns.load(Ordering::SeqCst) >= MAX_CONNS {
                        let mut s = stream;
                        http::respond_status(&mut s, 503, "Service Unavailable");
                        continue;
                    }
                    ctx2.conns.fetch_add(1, Ordering::SeqCst);
                    let ctx3 = ctx2.clone();
                    let _ =
                        std::thread::Builder::new().name("preview-conn".into()).spawn(move || {
                            handle_conn(stream, &ctx3);
                            ctx3.conns.fetch_sub(1, Ordering::SeqCst);
                        });
                }
            })?;
        Ok(PreviewServer {
            port: bound,
            app_port,
            started: Instant::now(),
            ctx,
            accept: Some(accept),
        })
    }

    /// The port actually bound (differs from the asked-for port only when 0
    /// was passed).
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn app_port(&self) -> u16 {
        self.app_port
    }

    /// How long this server has been up. Lets a reconciling caller give a
    /// freshly started server a grace period against state it races with (a
    /// run started but not yet registered anywhere the reconciler looks).
    pub fn age(&self) -> Duration {
        self.started.elapsed()
    }

    /// The URL the preview iframe should load: the proxied app.
    pub fn preview_url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    fn stop_inner(&mut self) {
        self.ctx.shutdown.store(true, Ordering::SeqCst);
        // Unblock the accept call so the thread observes the flag, and wake
        // parked pollers so their connections drain promptly.
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], self.port)),
            Duration::from_millis(200),
        );
        self.ctx.broker.wake.notify_all();
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PreviewServer {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

fn handle_conn(mut stream: TcpStream, ctx: &Ctx) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(60)));
    let Ok(read_half) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read_half);
    let req = match http::read_request(&mut reader) {
        Ok(r) => r,
        Err(_) => {
            http::respond_status(&mut stream, 400, "Bad Request");
            return;
        }
    };
    // A cross-site page in some browser must not be able to reach this server
    // at all — see origin_is_local. Loopback pages (the bridge, the preview
    // itself) and origin-less clients (agent CLIs) pass.
    if let Some(origin) = req.header("origin") {
        if !http::origin_is_local(origin) {
            http::respond_status(&mut stream, 403, "Forbidden");
            return;
        }
    }
    if req.path().starts_with(CONTROL_PREFIX) {
        control(&mut stream, &req, ctx);
    } else {
        proxy::forward(&mut stream, &req, ctx.app_port, &|e| ctx.broker.push_network(e));
    }
}

fn control(stream: &mut TcpStream, req: &http::Request, ctx: &Ctx) {
    let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
    let str_at = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    match (req.method.as_str(), req.path()) {
        ("POST", MCP_PATH) => match rpc::handle(&req.body) {
            rpc::Handled::Response(v) => http::respond_json(stream, 200, &v),
            rpc::Handled::Notification => http::respond_status(stream, 202, "Accepted"),
            rpc::Handled::Call { id, name, args } => {
                let response = exec_tool(id, &name, &args, ctx);
                http::respond_json(stream, 200, &response);
            }
        },
        // The optional SSE listen channel (and session teardown): this server
        // never pushes, so saying 405 here is the spec-sanctioned answer.
        (_, MCP_PATH) => {
            let _ =
                http::write_response(stream, 405, "Method Not Allowed", &[("allow", "POST")], b"");
        }
        ("GET", "/__agency__/bridge.js") => {
            let _ = http::write_response(
                stream,
                200,
                "OK",
                &[
                    ("content-type", "application/javascript"),
                    // The tag is re-fetched per document; never let a stale
                    // cached copy outlive an Agency update.
                    ("cache-control", "no-store"),
                ],
                include_bytes!("bridge.js"),
            );
        }
        ("POST", "/__agency__/hello") => {
            if let Some(instance) = str_at("instance") {
                ctx.broker.hello(&instance, str_at("url"), str_at("title"));
            }
            http::respond_json(stream, 200, &json!({}));
        }
        ("POST", "/__agency__/poll") => {
            let reply = match str_at("instance") {
                Some(instance) => ctx.broker.poll(&instance),
                None => json!({ "retire": true }),
            };
            http::respond_json(stream, 200, &reply);
        }
        ("POST", "/__agency__/result") => {
            if let (Some(id), Some(result)) =
                (body.get("id").and_then(Value::as_u64), body.get("result"))
            {
                ctx.broker.result(id, result.clone());
            }
            http::respond_json(stream, 200, &json!({}));
        }
        ("POST", "/__agency__/log") => {
            if let (Some(instance), Some(entries)) =
                (str_at("instance"), body.get("entries").and_then(Value::as_array))
            {
                let entries = entries
                    .iter()
                    .map(|e| ConsoleEntry {
                        level: e.get("level").and_then(Value::as_str).unwrap_or("log").to_string(),
                        text: e.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                    })
                    .collect();
                ctx.broker.log(&instance, entries);
            }
            http::respond_json(stream, 200, &json!({}));
        }
        _ => http::respond_status(stream, 404, "Not Found"),
    }
}

/// Execute one MCP tool call and shape its response. Read tools answer from
/// the buffers; act tools round-trip through the bridge.
fn exec_tool(id: Value, name: &str, args: &Value, ctx: &Ctx) -> Value {
    let arg = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match name {
        "preview_console" | "preview_network" => {
            let mut st = ctx.broker.state.lock().unwrap();
            let connected = Broker::bridge_alive(&st);
            let mut text = if name == "preview_console" {
                let (entries, missed) = st.console.drain_new();
                format_console(&entries, missed)
            } else {
                let (entries, missed) = st.network.drain_new();
                format_network(&entries, missed)
            };
            drop(st);
            if !connected {
                let note = guidance(ctx.app_port, &(ctx.hooks.facts)(), serving(ctx.app_port));
                text.push_str(&format!("\n\nNote: the preview is not connected. {note}"));
            }
            rpc::tool_text(id, &text, false)
        }
        "preview_screenshot" => match (ctx.hooks.screenshot)() {
            Ok(png) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(png);
                let at = ctx
                    .broker
                    .state
                    .lock()
                    .unwrap()
                    .url
                    .clone()
                    .unwrap_or_else(|| "the preview".to_string());
                rpc::tool_image(id, &b64, &format!("The preview pane in Agency, showing {at}."))
            }
            Err(reason) => rpc::tool_text(id, &reason, true),
        },
        "preview_snapshot" => bridge_tool(id, name, json!({ "kind": "snapshot" }), ctx),
        "preview_navigate" => {
            let path = arg("path");
            if path.is_empty() {
                return rpc::error(id, -32602, "preview_navigate needs a path");
            }
            bridge_tool(id, name, json!({ "kind": "navigate", "path": path }), ctx)
        }
        "preview_click" => {
            let selector = arg("selector");
            if selector.is_empty() {
                return rpc::error(id, -32602, "preview_click needs a selector");
            }
            bridge_tool(id, name, json!({ "kind": "click", "selector": selector }), ctx)
        }
        "preview_type" => {
            let selector = arg("selector");
            if selector.is_empty() {
                return rpc::error(id, -32602, "preview_type needs a selector");
            }
            let cmd = json!({
                "kind": "type",
                "selector": selector,
                "text": arg("text"),
                "clear": args.get("clear").and_then(Value::as_bool).unwrap_or(true),
                "enter": args.get("enter").and_then(Value::as_bool).unwrap_or(false),
            });
            bridge_tool(id, name, cmd, ctx)
        }
        _ => rpc::error(id, -32602, &format!("unknown tool: {name}")),
    }
}

fn bridge_tool(id: Value, tool: &str, payload: Value, ctx: &Ctx) -> Value {
    match ctx.broker.command(ctx, payload) {
        Ok(v) => match v.get("error").and_then(Value::as_str) {
            Some(err) => rpc::tool_text(id, err, true),
            None => rpc::tool_text(id, &format_bridge_result(tool, &v), false),
        },
        Err(reason) => rpc::tool_text(id, &reason, true),
    }
}

/// Turn a bridge result object into the sentence-or-outline an agent reads.
fn format_bridge_result(tool: &str, v: &Value) -> String {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
    match tool {
        "preview_snapshot" => {
            format!("{} \"{}\"\n\n{}", s("url"), s("title"), s("outline"))
        }
        "preview_navigate" => format!(
            "Navigation started to {}. Take a preview_snapshot once it loads, and read \
             preview_console for anything the page reported.",
            s("navigating")
        ),
        "preview_click" => format!(
            "Clicked {}. The page is now at {} (\"{}\"). Read preview_console for what it did.",
            s("clicked"),
            s("url"),
            s("title"),
        ),
        "preview_type" => format!("Typed into {}. The page is at {}.", s("typed_into"), s("url")),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn hooks(script: Option<(&str, &str)>, shot: Result<Vec<u8>, &str>) -> Hooks {
        let script = script.map(|(n, c)| (n.to_string(), c.to_string()));
        let shot = shot.map_err(str::to_string);
        Hooks {
            facts: Arc::new(move || Facts { script: script.clone() }),
            screenshot: Arc::new(move || shot.clone()),
        }
    }

    /// Raw HTTP client good enough for our own server.
    fn post(port: u16, path: &str, body: &str) -> (u16, String) {
        request(port, "POST", path, body, &[])
    }

    fn request(
        port: u16,
        method: &str,
        path: &str,
        body: &str,
        extra: &[(&str, &str)],
    ) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut head = format!("{method} {path} HTTP/1.1\r\nhost: x\r\n");
        for (n, v) in extra {
            head.push_str(&format!("{n}: {v}\r\n"));
        }
        head.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
        s.write_all(head.as_bytes()).unwrap();
        s.write_all(body.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let status: u16 = out.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    fn rpc_call(port: u16, msg: &str) -> Value {
        let (status, body) = post(port, MCP_PATH, msg);
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    fn tool_call(port: u16, name: &str, args: Value) -> Value {
        rpc_call(
            port,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})
                .to_string(),
        )
    }

    fn dead_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn start(hooks: Hooks) -> PreviewServer {
        PreviewServer::start(0, dead_port(), hooks).unwrap()
    }

    #[test]
    fn mcp_port_is_the_last_of_the_block_and_needs_room() {
        assert_eq!(mcp_port(5240, 10), Some(5249));
        assert_eq!(mcp_port(4000, 2), Some(4001));
        assert_eq!(mcp_port(5240, 1), None, "a one-port block is the app's");
        assert_eq!(mcp_port(5240, 0), None);
        assert_eq!(mcp_port(u16::MAX, 10), None, "no wrapping into low ports");
    }

    #[test]
    fn server_entry_is_a_remote_http_server() {
        let e = server_entry(5249);
        assert_eq!(e.name, SERVER_NAME);
        assert_eq!(e.url.as_deref(), Some("http://127.0.0.1:5249/__agency__/mcp"));
        assert_eq!(e.effective_transport(), crate::mcp::McpTransport::Http);
        assert!(e.validate().is_ok());
    }

    #[test]
    fn log_cursor_reports_new_entries_and_evictions() {
        let mut log: Log<u32> = Log::new();
        log.push(1);
        log.push(2);
        let (got, missed) = log.drain_new();
        assert_eq!((got, missed), (vec![1, 2], 0));
        let (got, missed) = log.drain_new();
        assert_eq!((got, missed), (vec![], 0));
        for i in 0..(LOG_CAP as u32 + 5) {
            log.push(i);
        }
        let (got, missed) = log.drain_new();
        assert_eq!(got.len(), LOG_CAP);
        assert_eq!(missed, 5);
    }

    #[test]
    fn initialize_and_tools_list_over_the_wire() {
        let srv = start(hooks(None, Err("n/a")));
        let r = rpc_call(
            srv.port(),
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        );
        assert_eq!(r["result"]["serverInfo"]["name"], SERVER_NAME);
        let (status, _) =
            post(srv.port(), MCP_PATH, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert_eq!(status, 202);
        let r = rpc_call(srv.port(), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), rpc::TOOL_NAMES.len());
    }

    #[test]
    fn cross_site_origins_are_refused_everywhere() {
        let srv = start(hooks(None, Err("n/a")));
        let (status, _) = request(
            srv.port(),
            "POST",
            MCP_PATH,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            &[("origin", "https://evil.example")],
        );
        assert_eq!(status, 403);
        let (status, _) =
            request(srv.port(), "GET", "/", "", &[("origin", "https://evil.example")]);
        assert_eq!(status, 403);
        // Loopback origins (the bridge's own fetches) pass.
        let (status, _) = request(
            srv.port(),
            "POST",
            MCP_PATH,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            &[("origin", &format!("http://127.0.0.1:{}", srv.port()))],
        );
        assert_eq!(status, 200);
    }

    #[test]
    fn sse_get_is_405_and_unknown_control_paths_404() {
        let srv = start(hooks(None, Err("n/a")));
        let (status, _) = request(srv.port(), "GET", MCP_PATH, "", &[]);
        assert_eq!(status, 405);
        let (status, _) = request(srv.port(), "GET", "/__agency__/nope", "", &[]);
        assert_eq!(status, 404);
        let (status, body) = request(srv.port(), "GET", "/__agency__/bridge.js", "", &[]);
        assert_eq!(status, 200);
        assert!(body.contains("__agencyPreviewBridge"));
    }

    #[test]
    fn tools_error_with_start_guidance_when_nothing_serves() {
        let srv = start(hooks(Some(("dev", "pnpm dev --port $AGENCY_PORT")), Err("n/a")));
        let r = tool_call(srv.port(), "preview_click", json!({"selector": "#go"}));
        assert_eq!(r["result"]["isError"], true);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("pnpm dev --port $AGENCY_PORT"), "{text}");
        assert!(text.contains(&format!("AGENCY_PORT={}", srv.app_port())), "{text}");

        // Console still answers (not an error), with the same note attached.
        let r = tool_call(srv.port(), "preview_console", json!({}));
        assert_eq!(r["result"]["isError"], false);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("No console output"), "{text}");
        assert!(text.contains("not connected"), "{text}");
    }

    #[test]
    fn console_log_flows_from_bridge_to_tool_with_cursor() {
        let srv = start(hooks(None, Err("n/a")));
        post(
            srv.port(),
            "/__agency__/hello",
            &json!({"instance": "b1", "url": "http://x/", "title": "T"}).to_string(),
        );
        post(
            srv.port(),
            "/__agency__/log",
            &json!({"instance": "b1", "entries": [{"level": "error", "text": "boom at app.js:3"}]})
                .to_string(),
        );
        let r = tool_call(srv.port(), "preview_console", json!({}));
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("[error] boom at app.js:3"), "{text}");
        // Entries from a retired instance are dropped.
        post(
            srv.port(),
            "/__agency__/log",
            &json!({"instance": "old", "entries": [{"level": "log", "text": "stale"}]}).to_string(),
        );
        let r = tool_call(srv.port(), "preview_console", json!({}));
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("No console output"), "{text}");
    }

    #[test]
    fn a_new_hello_retires_the_previous_instance() {
        let srv = start(hooks(None, Err("n/a")));
        post(srv.port(), "/__agency__/hello", &json!({"instance": "a"}).to_string());
        post(srv.port(), "/__agency__/hello", &json!({"instance": "b"}).to_string());
        let (_, body) = post(srv.port(), "/__agency__/poll", &json!({"instance": "a"}).to_string());
        assert!(body.contains("retire"), "{body}");
    }

    #[test]
    fn a_bridge_command_round_trips_through_poll_and_result() {
        let srv = start(hooks(None, Err("n/a")));
        let port = srv.port();
        post(
            port,
            "/__agency__/hello",
            &json!({"instance": "b1", "url": "http://x/", "title": "T"}).to_string(),
        );
        // The simulated page: poll until the click command arrives, execute,
        // post the result.
        let bridge = std::thread::spawn(move || loop {
            let (_, body) = post(port, "/__agency__/poll", &json!({"instance": "b1"}).to_string());
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            if let Some(id) = v.get("id").and_then(Value::as_u64) {
                assert_eq!(v["cmd"]["kind"], "click");
                assert_eq!(v["cmd"]["selector"], "#go");
                post(
                    port,
                    "/__agency__/result",
                    &json!({"instance": "b1", "id": id, "result": {"ok": true, "clicked": "button#go", "url": "http://x/done", "title": "Done"}}).to_string(),
                );
                break;
            }
        });
        let r = tool_call(port, "preview_click", json!({"selector": "#go"}));
        bridge.join().unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Clicked button#go"), "{text}");
        assert!(text.contains("http://x/done"), "{text}");
    }

    #[test]
    fn bridge_errors_come_back_as_tool_errors() {
        let srv = start(hooks(None, Err("n/a")));
        let port = srv.port();
        post(port, "/__agency__/hello", &json!({"instance": "b1"}).to_string());
        let bridge = std::thread::spawn(move || loop {
            let (_, body) = post(port, "/__agency__/poll", &json!({"instance": "b1"}).to_string());
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            if let Some(id) = v.get("id").and_then(Value::as_u64) {
                post(
                    port,
                    "/__agency__/result",
                    &json!({"instance": "b1", "id": id, "result": {"error": "nothing matches #gone"}}).to_string(),
                );
                break;
            }
        });
        let r = tool_call(port, "preview_click", json!({"selector": "#gone"}));
        bridge.join().unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["text"], "nothing matches #gone");
    }

    #[test]
    fn screenshot_uses_the_native_hook_and_reports_its_refusals() {
        let srv = start(hooks(None, Ok(vec![1, 2, 3])));
        let r = tool_call(srv.port(), "preview_screenshot", json!({}));
        assert_eq!(r["result"]["content"][1]["data"], "AQID");
        assert_eq!(r["result"]["content"][1]["mimeType"], "image/png");

        let srv = start(hooks(None, Err("the preview pane is not on screen")));
        let r = tool_call(srv.port(), "preview_screenshot", json!({}));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["text"], "the preview pane is not on screen");
    }

    #[test]
    fn proxied_traffic_lands_in_the_network_tool() {
        // Full path: a real upstream, a request through the proxy side of the
        // server, then the network tool reads the entry.
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = upstream.accept() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}",
                );
            }
        });
        let srv = PreviewServer::start(0, upstream_port, hooks(None, Err("n/a"))).unwrap();
        let (status, _) = request(srv.port(), "GET", "/api/x", "", &[]);
        assert_eq!(status, 200);
        let r = tool_call(srv.port(), "preview_network", json!({}));
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("GET /api/x → 200 application/json"), "{text}");
    }

    #[test]
    fn formatting_helpers_read_like_sentences() {
        assert_eq!(format_console(&[], 0), "No console output since the last read.");
        let entries = vec![ConsoleEntry { level: "warn".into(), text: "x".into() }];
        assert_eq!(
            format_console(&entries, 2),
            "(2 earlier entries dropped past the buffer)\n[warn] x"
        );
        let net = vec![NetworkEntry {
            method: "GET".into(),
            path: "/a.js".into(),
            status: 200,
            content_type: "text/javascript".into(),
            bytes: 2048,
            ms: 12,
        }];
        assert_eq!(format_network(&net, 0), "GET /a.js → 200 text/javascript, 2.0 kB, 12 ms");
        assert_eq!(human_bytes(3), "3 B");
        assert_eq!(human_bytes(2_097_152), "2.0 MB");
    }

    #[test]
    fn guidance_names_the_actual_command_and_port() {
        let facts = Facts { script: Some(("dev".into(), "pnpm dev".into())) };
        let g = guidance(5240, &facts, false);
        assert!(g.contains("`pnpm dev`"), "{g}");
        assert!(g.contains("AGENCY_PORT=5240"), "{g}");
        let g = guidance(5240, &Facts::default(), false);
        assert!(g.contains("no web run script"), "{g}");
        let g = guidance(5240, &facts, true);
        assert!(g.contains("has not connected"), "{g}");
    }

    /// House rule, same as the skills kit's: agent-facing generated copy
    /// carries no em dashes and stays ASCII. Checked over everything a tool
    /// call can put in front of an agent: guidance, formatted results, the
    /// tool list and the initialize instructions.
    #[test]
    fn agent_facing_copy_follows_the_house_style() {
        let facts = Facts { script: Some(("dev".into(), "pnpm dev".into())) };
        let mut copy = vec![
            guidance(5240, &facts, false),
            guidance(5240, &Facts::default(), false),
            guidance(5240, &facts, true),
            format_bridge_result(
                "preview_click",
                &json!({"clicked": "a", "url": "http://x", "title": "t"}),
            ),
            format_bridge_result("preview_navigate", &json!({"navigating": "http://x"})),
            format_bridge_result("preview_type", &json!({"typed_into": "a", "url": "http://x"})),
            format_bridge_result(
                "preview_snapshot",
                &json!({"url": "u", "title": "t", "outline": "o"}),
            ),
            format_console(&[], 0),
            format_network(&[], 0),
        ];
        let srv = start(hooks(None, Err("n/a")));
        let init =
            rpc_call(srv.port(), r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
        copy.push(init["result"]["instructions"].as_str().unwrap().to_string());
        let tools = rpc_call(srv.port(), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        copy.push(tools["result"].to_string());
        for text in copy {
            assert!(!text.contains('\u{2014}'), "em dash in {text:?}");
            assert!(text.is_ascii(), "non-ascii in {text:?}");
        }
    }

    /// The bridge ships as a string inside this binary and nothing else ever
    /// parses it, so a stray syntax error would surface only as a silently
    /// dead preview. Parse it with node where node exists (any machine that
    /// builds the frontend); elsewhere there is nothing to check with.
    #[test]
    fn the_bridge_script_parses_under_node() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.js");
        std::fs::write(&path, include_bytes!("bridge.js")).unwrap();
        let Ok(out) = std::process::Command::new("node").arg("--check").arg(&path).output() else {
            return;
        };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn stop_frees_the_port() {
        let srv = start(hooks(None, Err("n/a")));
        let port = srv.port();
        drop(srv);
        // The accept thread has exited; connections now fail (possibly after
        // one accepted-then-dropped wake connection).
        std::thread::sleep(Duration::from_millis(50));
        let refused = TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(200),
        )
        .is_err();
        assert!(refused, "the listener should be gone after drop");
    }
}

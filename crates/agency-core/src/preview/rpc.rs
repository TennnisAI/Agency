//! The MCP protocol layer of the preview server: JSON-RPC over streamable
//! HTTP, reduced to the stateless subset this server needs. Everything here is
//! a pure function from a request message to what should happen, so the whole
//! protocol surface is testable without a socket.
//!
//! The server is deliberately stateless in MCP terms: no session ids, no
//! server-initiated messages, no SSE stream. A client POSTs a message and gets
//! one JSON response (or 202 for a notification); a GET for the SSE channel is
//! answered 405, which the spec allows a server that has nothing to push.
//!
//! Tool names and descriptions are static strings on purpose: the tool list
//! lands in the agent's context, and anything volatile in it (ports, script
//! names) would break prompt-cache stability. Volatile facts travel in tool
//! *results* instead — see `guidance` in the parent module.

use serde_json::{json, Value};

/// Protocol revisions this server behaves correctly under. The negotiation
/// rule: echo the client's requested version when we know it, else answer with
/// the newest we support and let the client decide.
const PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
const LATEST_PROTOCOL: &str = "2025-06-18";

/// What one POSTed message asks of the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Handled {
    /// A complete JSON-RPC response, ready to send.
    Response(Value),
    /// A notification: nothing to answer beyond HTTP 202.
    Notification,
    /// A `tools/call` the caller must execute against the preview.
    Call { id: Value, name: String, args: Value },
}

/// Route one raw POST body. Malformed JSON and batches produce complete error
/// responses rather than errors, so the HTTP layer can always just send what
/// comes back.
pub fn handle(body: &[u8]) -> Handled {
    let msg: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return Handled::Response(error(Value::Null, -32700, &format!("parse error: {e}")))
        }
    };
    if msg.is_array() {
        // JSON-RPC batching predates streamable HTTP and was removed from the
        // 2025-06-18 revision; no client we serve sends it.
        return Handled::Response(error(Value::Null, -32600, "batch requests are not supported"));
    }
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

    // No id → notification; nothing is ever sent in reply.
    let Some(id) = id.filter(|v| !v.is_null()) else {
        return Handled::Notification;
    };

    match method.as_str() {
        "initialize" => Handled::Response(result(id, initialize_result(&params))),
        "ping" => Handled::Response(result(id, json!({}))),
        "tools/list" => Handled::Response(result(id, json!({ "tools": tools() }))),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !TOOL_NAMES.contains(&name.as_str()) {
                return Handled::Response(error(id, -32602, &format!("unknown tool: {name}")));
            }
            Handled::Call { id, name, args }
        }
        // Optional capability listings a client may probe for.
        "resources/list" => Handled::Response(result(id, json!({ "resources": [] }))),
        "prompts/list" => Handled::Response(result(id, json!({ "prompts": [] }))),
        _ => Handled::Response(error(id, -32601, &format!("method not found: {method}"))),
    }
}

fn initialize_result(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version =
        if PROTOCOL_VERSIONS.contains(&asked) { asked.to_string() } else { LATEST_PROTOCOL.into() };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "agency-preview",
            "title": "Agency preview",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "These tools see and drive the live preview of this workspace's web app \
            inside Agency, the desktop app this run was dispatched from. The preview renders \
            whatever the workspace's web run script serves on the workspace's port, and the user \
            sees the same pane you are driving. Take a preview_snapshot before clicking or \
            typing, and read preview_console after acting instead of assuming an action worked.",
    })
}

pub const TOOL_NAMES: &[&str] = &[
    "preview_console",
    "preview_network",
    "preview_snapshot",
    "preview_screenshot",
    "preview_navigate",
    "preview_click",
    "preview_type",
];

fn tools() -> Value {
    let none = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    json!([
        {
            "name": "preview_console",
            "description": "Console output from the preview page since the last call to this \
                tool: console.log/info/warn/error, uncaught errors, and unhandled promise \
                rejections. Read it after loading or interacting with a page to see what the \
                page actually reported.",
            "inputSchema": none,
        },
        {
            "name": "preview_network",
            "description": "Requests the preview made since the last call to this tool, with \
                method, path, status, size and duration. Covers everything the preview loads \
                from the app under development: documents, assets, and same-origin fetch/XHR.",
            "inputSchema": none,
        },
        {
            "name": "preview_snapshot",
            "description": "The preview's current page as structured text: URL, title, and an \
                outline of visible headings, links, buttons and form fields, each with a CSS \
                selector usable with preview_click and preview_type. The reliable way to see \
                the page; use it before clicking or typing.",
            "inputSchema": none,
        },
        {
            "name": "preview_screenshot",
            "description": "A pixel screenshot of the preview pane exactly as the user sees it \
                in Agency. Needs the preview pane to be on screen in the app; when it is not, \
                preview_snapshot still works.",
            "inputSchema": none,
        },
        {
            "name": "preview_navigate",
            "description": "Navigate the preview to a path within the app under development, \
                e.g. `/` or `/settings?tab=2`. Scoped to the app's own origin; anything else \
                is refused.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path (and query/hash) to open, resolved against the app's origin." },
                },
                "required": ["path"],
                "additionalProperties": false,
            },
        },
        {
            "name": "preview_click",
            "description": "Click the first element matching a CSS selector in the preview. \
                Take a preview_snapshot first to find selectors.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector of the element to click." },
                },
                "required": ["selector"],
                "additionalProperties": false,
            },
        },
        {
            "name": "preview_type",
            "description": "Type into the input, textarea or contenteditable element matching \
                a CSS selector. Replaces the current contents unless clear is false; enter \
                presses Enter afterwards.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector of the field." },
                    "text": { "type": "string", "description": "Text to type." },
                    "clear": { "type": "boolean", "description": "Replace the field's contents first (default true)." },
                    "enter": { "type": "boolean", "description": "Press Enter after typing (default false)." },
                },
                "required": ["selector", "text"],
                "additionalProperties": false,
            },
        },
    ])
}

pub fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A tool response carrying text. `is_error` is MCP's "the tool ran and is
/// telling you it failed" flag — still a successful JSON-RPC response.
pub fn tool_text(id: Value, text: &str, is_error: bool) -> Value {
    result(id, json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
}

/// A tool response carrying one image plus a caption line.
pub fn tool_image(id: Value, base64_png: &str, caption: &str) -> Value {
    result(
        id,
        json!({
            "content": [
                { "type": "text", "text": caption },
                { "type": "image", "data": base64_png, "mimeType": "image/png" },
            ],
            "isError": false,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(body: &str) -> Handled {
        handle(body.as_bytes())
    }

    #[test]
    fn initialize_echoes_known_versions_and_pins_unknown_ones() {
        let Handled::Response(r) = call(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        ) else {
            panic!("expected response");
        };
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "agency-preview");
        assert!(r["result"]["capabilities"]["tools"].is_object());

        let Handled::Response(r) = call(
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"9999-01-01"}}"#,
        ) else {
            panic!("expected response");
        };
        assert_eq!(r["result"]["protocolVersion"], LATEST_PROTOCOL);
    }

    #[test]
    fn tools_list_names_every_tool_once() {
        let Handled::Response(r) = call(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#) else {
            panic!("expected response");
        };
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, TOOL_NAMES);
        // Every tool must carry a schema, or some clients refuse the server.
        for t in r["result"]["tools"].as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        }
    }

    #[test]
    fn notifications_get_nothing_and_calls_are_routed() {
        assert_eq!(
            call(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Handled::Notification
        );
        let Handled::Call { id, name, args } = call(
            r##"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"preview_click","arguments":{"selector":"#go"}}}"##,
        ) else {
            panic!("expected call");
        };
        assert_eq!(id, json!(7));
        assert_eq!(name, "preview_click");
        assert_eq!(args["selector"], "#go");
    }

    #[test]
    fn unknown_tool_method_and_garbage_become_json_rpc_errors() {
        let Handled::Response(r) = call(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell_exec"}}"#,
        ) else {
            panic!("expected response");
        };
        assert_eq!(r["error"]["code"], -32602);

        let Handled::Response(r) = call(r#"{"jsonrpc":"2.0","id":1,"method":"wat"}"#) else {
            panic!("expected response");
        };
        assert_eq!(r["error"]["code"], -32601);

        let Handled::Response(r) = call("not json") else { panic!("expected response") };
        assert_eq!(r["error"]["code"], -32700);

        let Handled::Response(r) = call(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#) else {
            panic!("expected response");
        };
        assert_eq!(r["error"]["code"], -32600);
    }

    #[test]
    fn ping_and_empty_capability_lists_answer() {
        let Handled::Response(r) = call(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#) else {
            panic!("expected response");
        };
        assert!(r["result"].as_object().unwrap().is_empty());
        let Handled::Response(r) = call(r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#)
        else {
            panic!("expected response");
        };
        assert_eq!(r["result"]["resources"], json!([]));
    }

    #[test]
    fn tool_content_helpers_shape_mcp_results() {
        let t = tool_text(json!(1), "hello", true);
        assert_eq!(t["result"]["isError"], true);
        assert_eq!(t["result"]["content"][0]["text"], "hello");
        let i = tool_image(json!(2), "QUJD", "the pane");
        assert_eq!(i["result"]["content"][1]["mimeType"], "image/png");
        assert_eq!(i["result"]["content"][1]["data"], "QUJD");
        assert_eq!(i["result"]["content"][0]["text"], "the pane");
    }
}

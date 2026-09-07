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

use super::Caps;
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
///
/// `caps` says which tool groups this run is actually serving, read fresh per
/// request: the user can switch either off in Agency while an agent is mid-run,
/// and a tool that is no longer there must stop being listed and stop
/// answering. It only ever subtracts — the descriptions themselves are static,
/// so what does remain in the agent's context stays byte-identical.
pub fn handle(body: &[u8], caps: Caps) -> Handled {
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
        "tools/list" => Handled::Response(result(id, json!({ "tools": tools(caps) }))),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !enabled(&name, caps) {
                // A tool the user has switched off is not the same mistake as a
                // tool that never existed, and an agent that hears "unknown"
                // for one goes looking for a typo instead of asking.
                return Handled::Response(match switched_off_reason(&name) {
                    Some(why) => error(id, -32602, why),
                    None => error(id, -32602, &format!("unknown tool: {name}")),
                });
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
        "instructions": "These tools reach into Agency, the desktop app this run was dispatched \
            from, for two things the workspace on disk cannot tell you. The preview tools see and \
            drive the live preview of this workspace's web app: it renders whatever the \
            workspace's web run script serves on the workspace's port, and the user sees the same \
            pane you are driving, so take a preview_snapshot before clicking or typing and read \
            preview_console after acting instead of assuming an action worked. editor_open_file \
            says which file the user has open in front of them right now, which is what \
            \"this file\" and \"here\" mean when they say it. Whichever of the two the user has \
            switched off is not listed.",
    })
}

/// The preview half: everything that needs a page to act on.
pub const PREVIEW_TOOLS: &[&str] = &[
    "preview_console",
    "preview_network",
    "preview_snapshot",
    "preview_screenshot",
    "preview_navigate",
    "preview_click",
    "preview_type",
];

/// The editor half: what Agency's own window is showing (AGE-200).
pub const EDITOR_TOOLS: &[&str] = &["editor_open_file"];

/// Whether `name` is a tool this server is serving right now.
pub fn enabled(name: &str, caps: Caps) -> bool {
    (caps.preview && PREVIEW_TOOLS.contains(&name)) || (caps.editor && EDITOR_TOOLS.contains(&name))
}

/// Why a real tool is not answering, when the reason is a switch rather than a
/// typo. `None` for a name no version of this server has ever had.
fn switched_off_reason(name: &str) -> Option<&'static str> {
    if PREVIEW_TOOLS.contains(&name) {
        Some(
            "the preview tools are switched off for this project: `agent_tools = false` under \
              [preview] in .agency/agency.toml, or the project has no web run script for a \
              preview to render. Ask the user to turn them on in Agency.",
        )
    } else if EDITOR_TOOLS.contains(&name) {
        Some(
            "editor_open_file is switched off: Agency shares the file the user has open only \
              when \"Let agents see the file you have open\" is on in its settings, and it is off \
              by default. Ask the user which file they mean, or ask them to turn it on.",
        )
    } else {
        None
    }
}

fn tools(caps: Caps) -> Value {
    let none = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    let all = json!([
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
        {
            "name": "editor_open_file",
            "description": "The file the user has open in front of them in Agency right now, \
                with the path and whether it is this workspace's copy or the project's own \
                checkout. This is what \"this file\", \"this note\" and \"here\" refer to when the \
                user says one of them without naming a file. Call it then, and before asking \
                which file they mean; the answer is a live reading, so ask again rather than \
                relying on an earlier one.",
            "inputSchema": none,
        },
    ]);
    Value::Array(
        all.as_array()
            .expect("tools() builds an array")
            .iter()
            .filter(|t| enabled(t["name"].as_str().unwrap_or_default(), caps))
            .cloned()
            .collect(),
    )
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

    const ALL: Caps = Caps { preview: true, editor: true };

    fn call(body: &str) -> Handled {
        handle(body.as_bytes(), ALL)
    }

    fn tool_names(caps: Caps) -> Vec<String> {
        let Handled::Response(r) =
            handle(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, caps)
        else {
            panic!("expected response");
        };
        r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
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
        let every: Vec<&str> = PREVIEW_TOOLS.iter().chain(EDITOR_TOOLS.iter()).copied().collect();
        assert_eq!(names, every);
        // Every tool must carry a schema, or some clients refuse the server.
        for t in r["result"]["tools"].as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        }
    }

    #[test]
    fn a_switched_off_half_is_not_listed() {
        assert_eq!(tool_names(Caps { preview: true, editor: false }), PREVIEW_TOOLS);
        assert_eq!(tool_names(Caps { preview: false, editor: true }), EDITOR_TOOLS);
        assert!(tool_names(Caps::none()).is_empty());
    }

    #[test]
    fn descriptions_do_not_move_when_the_other_half_is_switched_off() {
        // The tool list lands in the agent's context; only its length may
        // change with the switches, never a byte of what stays.
        let Handled::Response(all) = call(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        else {
            panic!("expected response");
        };
        let Handled::Response(half) = handle(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            Caps { preview: true, editor: false },
        ) else {
            panic!("expected response");
        };
        let all = all["result"]["tools"].as_array().unwrap();
        let half = half["result"]["tools"].as_array().unwrap();
        assert_eq!(&all[..half.len()], &half[..]);
    }

    #[test]
    fn a_switched_off_tool_says_so_instead_of_denying_it_exists() {
        let Handled::Response(r) = handle(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"editor_open_file"}}"#,
            Caps { preview: true, editor: false },
        ) else {
            panic!("expected response");
        };
        let msg = r["error"]["message"].as_str().unwrap();
        assert!(msg.contains("switched off"), "{msg}");
        assert!(!msg.contains("unknown tool"), "{msg}");

        let Handled::Response(r) = handle(
            br##"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"preview_click","arguments":{"selector":"#go"}}}"##,
            Caps { preview: false, editor: true },
        ) else {
            panic!("expected response");
        };
        assert!(r["error"]["message"].as_str().unwrap().contains("switched off"));
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

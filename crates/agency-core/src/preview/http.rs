//! Minimal HTTP/1.1 plumbing for the preview server.
//!
//! Hand-rolled over `std::net` because agency-core has no async runtime and no
//! HTTP dependency, and the traffic is loopback-only between three parties we
//! control: an agent CLI's MCP client, the preview page's bridge script, and
//! the dev server being proxied. Every connection carries one request and is
//! closed after the response (`Connection: close`), which keeps the state
//! machine trivial; on loopback the extra handshakes cost nothing measurable.

use anyhow::{bail, Result};
use std::io::{BufRead, Write};

/// Caps chosen for what actually flows here: MCP tool calls and bridge results
/// are small JSON, and the largest legitimate body is a file upload posted
/// through the proxied dev server.
const MAX_HEADER_LINE: usize = 16 * 1024;
const MAX_HEADERS: usize = 128;
const MAX_BODY: usize = 32 * 1024 * 1024;

/// One parsed request. Header names are lowercased at parse time so lookups
/// need no case dance.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Path plus query, exactly as the client sent it.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The path without its query string.
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or(&self.target)
    }
}

/// Read one request off the stream. Bodies are read whole, by Content-Length
/// only — none of our clients (MCP CLIs, `fetch` from the bridge, browsers
/// posting forms) send chunked request bodies, so a chunked request is refused
/// rather than half-supported.
pub fn read_request<R: BufRead>(reader: &mut R) -> Result<Request> {
    let line = read_line(reader)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let version = parts.next().unwrap_or_default();
    if method.is_empty() || target.is_empty() || !version.starts_with("HTTP/1") {
        bail!("malformed request line");
    }

    let mut headers = Vec::new();
    loop {
        let line = read_line(reader)?;
        if line.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            bail!("too many headers");
        }
        let Some((name, value)) = line.split_once(':') else {
            bail!("malformed header");
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }

    let header = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    if header("transfer-encoding").is_some_and(|t| !t.eq_ignore_ascii_case("identity")) {
        bail!("chunked request bodies are not supported");
    }
    let len: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if len > MAX_BODY {
        bail!("request body too large");
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(Request { method, target, headers, body })
}

/// One CRLF-terminated line, without its terminator.
fn read_line<R: BufRead>(reader: &mut R) -> Result<String> {
    let mut buf = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;
        if byte[0] == b'\n' {
            break;
        }
        if buf.len() >= MAX_HEADER_LINE {
            bail!("header line too long");
        }
        buf.push(byte[0]);
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Write a complete response and mark the connection for closing. `headers`
/// are extras beyond Content-Length/Connection (Content-Type, mostly).
pub fn write_response<W: Write>(
    w: &mut W,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (n, v) in headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", body.len()));
    w.write_all(head.as_bytes())?;
    w.write_all(body)?;
    w.flush()
}

pub fn respond_json<W: Write>(w: &mut W, status: u16, body: &serde_json::Value) {
    let bytes = body.to_string().into_bytes();
    let reason = if status == 200 { "OK" } else { "Error" };
    let _ = write_response(w, status, reason, &[("content-type", "application/json")], &bytes);
}

pub fn respond_status<W: Write>(w: &mut W, status: u16, reason: &str) {
    let _ = write_response(w, status, reason, &[], reason.as_bytes());
}

/// Whether an `Origin` header names this machine. Requests from the MCP client
/// carry no Origin at all; the bridge's `fetch` calls carry their own loopback
/// origin. Anything else is a browser page from somewhere on the network
/// trying to reach a loopback server it should not know exists — refused, so a
/// malicious web page cannot drive the preview through the user's browser.
/// `"null"` (sandboxed/opaque origins) is refused with the rest.
pub fn origin_is_local(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or(rest);
    let host = if let Some(v6) = host.strip_prefix('[') {
        v6.split(']').next().unwrap_or(v6)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(raw: &str) -> Result<Request> {
        read_request(&mut Cursor::new(raw.as_bytes().to_vec()))
    }

    #[test]
    fn parses_request_with_body_and_lowercases_header_names() {
        let req = parse(
            "POST /__agency__/mcp?x=1 HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 4\r\n\r\nabcd",
        )
        .unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.target, "/__agency__/mcp?x=1");
        assert_eq!(req.path(), "/__agency__/mcp");
        assert_eq!(req.header("content-type"), Some("application/json"));
        assert_eq!(req.body, b"abcd");
    }

    #[test]
    fn refuses_chunked_and_malformed_requests() {
        assert!(parse("POST / HTTP/1.1\r\ntransfer-encoding: chunked\r\n\r\n").is_err());
        assert!(parse("nonsense\r\n\r\n").is_err());
        assert!(parse("GET /\r\n\r\n").is_err(), "missing HTTP version");
    }

    #[test]
    fn origin_check_accepts_loopback_and_nothing_else() {
        for ok in [
            "http://localhost:5249",
            "http://127.0.0.1",
            "http://127.0.0.1:80/",
            "https://localhost",
            "http://[::1]:5249",
        ] {
            assert!(origin_is_local(ok), "{ok}");
        }
        for bad in [
            "https://evil.example",
            "http://192.168.1.4:5249",
            "null",
            "file://",
            "http://localhost.evil.example",
        ] {
            assert!(!origin_is_local(bad), "{bad}");
        }
    }

    #[test]
    fn write_response_carries_length_and_close() {
        let mut out = Vec::new();
        write_response(&mut out, 200, "OK", &[("content-type", "text/plain")], b"hi").unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("content-length: 2\r\n"));
        assert!(text.contains("connection: close\r\n"));
        assert!(text.ends_with("\r\n\r\nhi"));
    }
}

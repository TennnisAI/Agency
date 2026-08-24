//! The reverse proxy half of the preview server: everything outside
//! `/__agency__/` is forwarded to the workspace's dev server on loopback, with
//! one change — HTML documents get the bridge script injected, which is what
//! makes the preview drivable at all. The page and the bridge share an origin
//! (this proxy's), so the bridge has full DOM access without any browser
//! extension machinery, and every asset the page loads passes through here,
//! which is where the network log comes from.
//!
//! WebSocket upgrades (dev-server HMR) are spliced through as raw TCP after
//! the 101, so hot reload keeps working inside the proxied preview.

use super::http::{write_response, Request};
use super::NetworkEntry;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// The tag injected into proxied HTML. Served by the control side of the same
/// origin, parser-blocking on purpose: the console wrap must be in place
/// before the app's own scripts run, or their output is lost.
const BRIDGE_TAG: &[u8] = b"<script src=\"/__agency__/bridge.js\"></script>";

/// Hop-by-hop headers, plus the ones this proxy owns (framing, encoding).
/// Everything else — cookies, caching, CORS the dev server sets — passes.
const STRIPPED: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "accept-encoding",
    "host",
    "content-length",
];

/// Forward one request to the dev server on `app_port` and relay the response.
/// `log` receives one entry per request, including failures to connect — those
/// are exactly the entries an agent debugging "the page is blank" needs.
pub(crate) fn forward(
    client: &mut TcpStream,
    req: &Request,
    app_port: u16,
    log: &dyn Fn(NetworkEntry),
) {
    let started = Instant::now();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], app_port));
    let upstream = TcpStream::connect_timeout(&addr, Duration::from_secs(3));
    let mut upstream = match upstream {
        Ok(s) => s,
        Err(_) => {
            log(entry(req, 502, "-", 0, started));
            let body = not_running_page(app_port);
            let _ = write_response(
                client,
                502,
                "Bad Gateway",
                &[("content-type", "text/html; charset=utf-8"), ("cache-control", "no-store")],
                body.as_bytes(),
            );
            return;
        }
    };
    let _ = upstream.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = upstream.set_write_timeout(Some(Duration::from_secs(60)));

    let ws = is_websocket(req);
    if upstream.write_all(&upstream_head(req, app_port, ws)).is_err()
        || upstream.write_all(&req.body).is_err()
    {
        log(entry(req, 502, "-", 0, started));
        let _ = write_response(client, 502, "Bad Gateway", &[], b"upstream write failed");
        return;
    }

    let mut reader = BufReader::new(upstream);
    let Ok((status, reason, headers)) = read_response_head(&mut reader) else {
        log(entry(req, 502, "-", 0, started));
        let _ = write_response(client, 502, "Bad Gateway", &[], b"upstream sent no response");
        return;
    };
    let header = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n));
    let content_type = header("content-type").map(|(_, v)| v.clone()).unwrap_or_default();

    if ws && status == 101 {
        log(entry(req, 101, "websocket", 0, started));
        tunnel(client, reader, &reason, &headers);
        return;
    }

    if content_type.to_ascii_lowercase().contains("text/html") {
        // HTML is buffered whole so the bridge can be injected and the length
        // recomputed. Dev-server documents are small; the caps in http.rs
        // bound the worst case.
        let chunked = header("transfer-encoding")
            .is_some_and(|(_, v)| v.to_ascii_lowercase().contains("chunked"));
        let len = header("content-length").and_then(|(_, v)| v.parse::<usize>().ok());
        let body = match read_body(&mut reader, chunked, len) {
            Ok(b) => inject_bridge(&b),
            Err(_) => {
                log(entry(req, 502, "-", 0, started));
                let _ = write_response(client, 502, "Bad Gateway", &[], b"upstream body failed");
                return;
            }
        };
        let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
        for (n, v) in &headers {
            if !STRIPPED.contains(&n.to_ascii_lowercase().as_str()) {
                head.push_str(&format!("{n}: {v}\r\n"));
            }
        }
        // No frame-ancestors header, deliberately. The packaged app's origin
        // is the custom scheme `tauri://localhost`, and whether WebKit matches
        // a custom-scheme CSP source can only be proven in the running app; a
        // wrong guess here blocks Agency's own iframe and kills the whole
        // preview. A foreign page embedding this proxy gets nothing anyway:
        // same-origin policy keeps it from reading the frame, and the Origin
        // check in handle_conn refuses every programmatic cross-site request.
        head.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", body.len()));
        let wrote = client.write_all(head.as_bytes()).and_then(|_| client.write_all(&body));
        let _ = wrote;
        log(entry(req, status, &content_type, body.len() as u64, started));
        return;
    }

    // Everything else streams through unchanged (framing included), so large
    // assets and server-sent event streams behave as if the proxy were not
    // there. Upstream was asked to close when done, so EOF ends the copy.
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (n, v) in &headers {
        let lower = n.to_ascii_lowercase();
        if !STRIPPED.contains(&lower.as_str())
            || lower == "content-length"
            || lower == "transfer-encoding"
        {
            head.push_str(&format!("{n}: {v}\r\n"));
        }
    }
    head.push_str("connection: close\r\n\r\n");
    if client.write_all(head.as_bytes()).is_err() {
        return;
    }
    let copied = copy_until_eof(&mut reader, client);
    log(entry(req, status, &content_type, copied, started));
}

fn entry(
    req: &Request,
    status: u16,
    content_type: &str,
    bytes: u64,
    started: Instant,
) -> NetworkEntry {
    NetworkEntry {
        method: req.method.clone(),
        path: req.target.clone(),
        status,
        content_type: content_type.split(';').next().unwrap_or_default().trim().to_string(),
        bytes,
        ms: started.elapsed().as_millis() as u64,
    }
}

fn is_websocket(req: &Request) -> bool {
    req.header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
}

/// The request head sent upstream. Accept-Encoding is dropped so HTML arrives
/// uncompressed and injectable; assets lose compression too, which on loopback
/// costs nothing anyone can notice.
fn upstream_head(req: &Request, app_port: u16, ws: bool) -> Vec<u8> {
    let mut head =
        format!("{} {} HTTP/1.1\r\nhost: 127.0.0.1:{app_port}\r\n", req.method, req.target);
    for (n, v) in &req.headers {
        if !STRIPPED.contains(&n.as_str()) {
            head.push_str(&format!("{n}: {v}\r\n"));
        }
    }
    if ws {
        head.push_str("connection: upgrade\r\nupgrade: websocket\r\n");
    } else {
        head.push_str("connection: close\r\n");
    }
    if !req.body.is_empty() || req.header("content-length").is_some() {
        head.push_str(&format!("content-length: {}\r\n", req.body.len()));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

fn read_response_head<R: BufRead>(
    r: &mut R,
) -> std::io::Result<(u16, String, Vec<(String, String)>)> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.trim_end().splitn(3, ' ');
    let _version = parts.next().unwrap_or_default();
    let status: u16 = parts.next().unwrap_or_default().parse().unwrap_or(502);
    let reason = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        r.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((n, v)) = line.split_once(':') {
            headers.push((n.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok((status, reason, headers))
}

/// A response body, whichever way upstream framed it. `len` wins over EOF;
/// chunked wins over both.
fn read_body<R: BufRead>(r: &mut R, chunked: bool, len: Option<usize>) -> std::io::Result<Vec<u8>> {
    if chunked {
        let mut body = Vec::new();
        loop {
            let mut line = String::new();
            r.read_line(&mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16)
                .unwrap_or(0);
            if size == 0 {
                // Trailing headers (usually none), then the final blank line.
                loop {
                    let mut t = String::new();
                    r.read_line(&mut t)?;
                    if t.trim_end().is_empty() {
                        break;
                    }
                }
                return Ok(body);
            }
            let mut chunk = vec![0u8; size];
            r.read_exact(&mut chunk)?;
            body.extend_from_slice(&chunk);
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf)?;
        }
    }
    match len {
        Some(n) => {
            let mut body = vec![0u8; n];
            r.read_exact(&mut body)?;
            Ok(body)
        }
        None => {
            let mut body = Vec::new();
            r.read_to_end(&mut body)?;
            Ok(body)
        }
    }
}

/// Relay bytes until upstream closes. Read timeouts (a stalled dev server) end
/// the copy rather than looping forever; the client sees a truncated body,
/// which is what actually happened.
fn copy_until_eof<R: Read, W: Write>(from: &mut R, to: &mut W) -> u64 {
    let mut buf = [0u8; 16 * 1024];
    let mut total = 0u64;
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n as u64;
                if to.write_all(&buf[..n]).is_err() || to.flush().is_err() {
                    break;
                }
            }
        }
    }
    total
}

/// Splice a WebSocket connection: write the 101 head to the client, then copy
/// bytes both ways until either side closes. The client→upstream direction
/// gets its own thread; this thread carries upstream→client.
fn tunnel(
    client: &mut TcpStream,
    reader: BufReader<TcpStream>,
    reason: &str,
    headers: &[(String, String)],
) {
    let mut head = format!("HTTP/1.1 101 {reason}\r\n");
    for (n, v) in headers {
        let lower = n.to_ascii_lowercase();
        if lower != "content-length" {
            head.push_str(&format!("{n}: {v}\r\n"));
        }
    }
    head.push_str("\r\n");
    if client.write_all(head.as_bytes()).is_err() {
        return;
    }
    let upstream = reader.into_inner();
    // Long-lived idle tunnels are real (HMR sockets sit quiet between edits),
    // so no read timeouts here: the copy ends when a side closes, and both
    // ends live on loopback inside processes whose lifetime bounds ours.
    let _ = upstream.set_read_timeout(None);
    let _ = client.set_read_timeout(None);
    let (Ok(mut up_w), Ok(mut cl_r)) = (upstream.try_clone(), client.try_clone()) else {
        return;
    };
    std::thread::Builder::new()
        .name("preview-ws-up".into())
        .spawn(move || {
            copy_until_eof(&mut cl_r, &mut up_w);
            let _ = up_w.shutdown(std::net::Shutdown::Both);
        })
        .ok();
    let mut up_r = upstream;
    copy_until_eof(&mut up_r, client);
    let _ = client.shutdown(std::net::Shutdown::Both);
    let _ = up_r.shutdown(std::net::Shutdown::Both);
}

/// Insert the bridge tag into an HTML document: right after `<head…>` when
/// there is one, else after `<html…>`, else at the very top. Parser-blocking
/// placement at the head start is deliberate — see [`BRIDGE_TAG`].
pub(crate) fn inject_bridge(html: &[u8]) -> Vec<u8> {
    let insert_at = tag_end(html, b"<head").or_else(|| tag_end(html, b"<html")).unwrap_or(0);
    let mut out = Vec::with_capacity(html.len() + BRIDGE_TAG.len());
    out.extend_from_slice(&html[..insert_at]);
    out.extend_from_slice(BRIDGE_TAG);
    out.extend_from_slice(&html[insert_at..]);
    out
}

/// Byte offset just past the `>` of the first `tag` occurrence, matched
/// case-insensitively. The byte after the tag name must end the name (`>` or
/// whitespace), so `<head` never matches `<header`.
fn tag_end(html: &[u8], tag: &[u8]) -> Option<usize> {
    let lower: Vec<u8> = html.iter().map(|b| b.to_ascii_lowercase()).collect();
    let mut from = 0;
    while let Some(pos) = find(&lower[from..], tag).map(|p| p + from) {
        let after = lower.get(pos + tag.len());
        if matches!(after, Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')) {
            return lower[pos..].iter().position(|&b| b == b'>').map(|gt| pos + gt + 1);
        }
        from = pos + 1;
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// What the user (and the agent, via the network log) sees while nothing is
/// serving on the app's port yet. Refreshes itself so the pane recovers the
/// moment the dev server comes up, with no reload button hunting.
fn not_running_page(app_port: u16) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"refresh\" content=\"2\">\
         <title>Waiting for the app</title>\
         <style>body{{font:14px -apple-system,sans-serif;color:#888;display:flex;\
         align-items:center;justify-content:center;height:100vh;margin:0}}</style></head>\
         <body><div>Nothing is serving on port {app_port} yet. This pane reloads on its own \
         once the run script is up.</div></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn injects_after_head_html_or_at_top() {
        let s = |b: &[u8]| String::from_utf8(b.to_vec()).unwrap();
        let tag = s(BRIDGE_TAG);

        let with_head = inject_bridge(b"<!doctype html><HTML><HEAD lang=\"en\"><title>x</title>");
        assert_eq!(
            s(&with_head),
            format!("<!doctype html><HTML><HEAD lang=\"en\">{tag}<title>x</title>")
        );

        let html_only = inject_bridge(b"<html data-x><body></body></html>");
        assert_eq!(s(&html_only), format!("<html data-x>{tag}<body></body></html>"));

        let fragment = inject_bridge(b"<p>hi</p>");
        assert_eq!(s(&fragment), format!("{tag}<p>hi</p>"));

        // `<header>` must not read as `<head>`.
        let header_el = inject_bridge(b"<div><header>x</header></div>");
        assert_eq!(s(&header_el), format!("{tag}<div><header>x</header></div>"));
    }

    /// A single-request upstream: accepts one connection, sends `response`,
    /// records what it received.
    fn fake_upstream(response: &'static [u8]) -> (u16, Arc<Mutex<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            // One read is enough for these tests' small requests.
            let n = s.read(&mut buf).unwrap_or(0);
            seen2.lock().unwrap().extend_from_slice(&buf[..n]);
            let _ = s.write_all(response);
        });
        (port, seen)
    }

    /// Run `forward` against a pair of in-process sockets and return what the
    /// client got plus the network log.
    fn proxy_roundtrip(req: Request, app_port: u16) -> (Vec<u8>, Vec<NetworkEntry>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut client = TcpStream::connect(addr).unwrap();
            client.read_to_end(&mut out).unwrap();
            out
        });
        let (mut server_side, _) = listener.accept().unwrap();
        let log = Mutex::new(Vec::new());
        forward(&mut server_side, &req, app_port, &|e| log.lock().unwrap().push(e));
        drop(server_side);
        (t.join().unwrap(), log.into_inner().unwrap())
    }

    fn get(path: &str) -> Request {
        Request {
            method: "GET".into(),
            target: path.into(),
            headers: vec![("accept".into(), "*/*".into())],
            body: Vec::new(),
        }
    }

    #[test]
    fn html_is_injected_relengthed_and_logged() {
        let (port, seen) = fake_upstream(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: 28\r\n\r\n<html><body>hi</body></html>",
        );
        let (got, log) = proxy_roundtrip(get("/"), port);
        let text = String::from_utf8_lossy(&got);
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.contains("/__agency__/bridge.js"), "{text}");
        // Content-Length was recomputed to include the injected tag.
        let len: usize = text
            .lines()
            .find_map(|l| l.strip_prefix("content-length: "))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let body = &got[got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4..];
        assert_eq!(body.len(), len);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].status, 200);
        assert_eq!(log[0].content_type, "text/html");
        // The upstream request was rewritten: host set, close requested.
        let sent = String::from_utf8(seen.lock().unwrap().clone()).unwrap();
        assert!(sent.starts_with("GET / HTTP/1.1\r\n"), "{sent}");
        assert!(sent.contains(&format!("host: 127.0.0.1:{port}")), "{sent}");
        assert!(sent.contains("connection: close"), "{sent}");
    }

    #[test]
    fn chunked_html_is_reassembled_before_injection() {
        let (port, _) = fake_upstream(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ntransfer-encoding: chunked\r\n\r\n6\r\n<html>\r\n7\r\n</html>\r\n0\r\n\r\n",
        );
        let (got, _) = proxy_roundtrip(get("/"), port);
        let text = String::from_utf8_lossy(&got);
        assert!(
            text.contains("<html><script src=\"/__agency__/bridge.js\"></script></html>"),
            "{text}"
        );
        assert!(!text.to_lowercase().contains("transfer-encoding"), "{text}");
    }

    #[test]
    fn non_html_streams_through_untouched() {
        let (port, _) = fake_upstream(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/javascript\r\ncontent-length: 12\r\n\r\nconsole.hi()",
        );
        let (got, log) = proxy_roundtrip(get("/app.js"), port);
        let text = String::from_utf8_lossy(&got);
        assert!(text.contains("console.hi()"), "{text}");
        assert!(!text.contains("bridge.js"), "{text}");
        assert_eq!(log[0].content_type, "application/javascript");
        assert_eq!(log[0].bytes, 12);
    }

    #[test]
    fn dead_upstream_serves_the_waiting_page_and_logs_it() {
        // A port nothing listens on: bind then drop to reserve a dead one.
        let dead = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (got, log) = proxy_roundtrip(get("/"), dead);
        let text = String::from_utf8_lossy(&got);
        assert!(text.starts_with("HTTP/1.1 502"), "{text}");
        assert!(text.contains(&format!("Nothing is serving on port {dead}")), "{text}");
        assert!(text.contains("http-equiv=\"refresh\""), "{text}");
        assert_eq!(log[0].status, 502);
    }
}

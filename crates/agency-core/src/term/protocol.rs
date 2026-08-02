//! IPC message types and the length-prefixed frame codec.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

// v2: request/reply messages carry a `seq` tag so a reply that arrives after
// its request timed out can be discarded instead of desyncing the channel.
//
// Bump this ONLY for a real wire change. A mismatch is not a polite upgrade
// signal: the app shuts the running daemon down and every live session on it
// dies. One socket is shared by the installed app and any dev build, so a bump
// on a branch means launching `./dev.sh` kills the agents running in the
// release app (and vice versa). Behaviour changes that are wire-compatible —
// e.g. a snapshot that restores more terminal modes — ride along with the next
// daemon start instead, and the client degrades gracefully until then.
pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SessionStatus {
    Running,
    Exited { code: i32 },
    Gone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackSpec {
    pub command: String,
    pub args: Vec<String>,
    pub grace_ms: u64,
}

// Request/reply variants carry `seq` (client-assigned, >= 1, echoed by the
// server) so the client can match replies to requests. `#[serde(default)]`
// keeps decoding tolerant of the seq-less v1 wire format: a legacy peer's
// message reads as seq 0, which the client treats as "untagged".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello { version: u32, #[serde(default)] seq: u64 },
    StartSession {
        id: String,
        cwd: String,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cols: u16,
        rows: u16,
        #[serde(default)]
        fallback: Option<FallbackSpec>,
        #[serde(default)]
        seq: u64,
    },
    Subscribe { id: String },
    Unsubscribe { id: String },
    Resize { id: String, cols: u16, rows: u16 },
    Capture { id: String, lines: usize, #[serde(default)] seq: u64 },
    Status { id: String, #[serde(default)] seq: u64 },
    Kill { id: String },
    List { #[serde(default)] seq: u64 },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMsg {
    Hello { version: u32, #[serde(default)] seq: u64 },
    Started { id: String, #[serde(default)] seq: u64 },
    Captured { id: String, text: String, #[serde(default)] seq: u64 },
    Status { id: String, status: SessionStatus, #[serde(default)] seq: u64 },
    List { sessions: Vec<(String, SessionStatus)>, #[serde(default)] seq: u64 },
    Exited { id: String, code: i32 },
    Error { id: Option<String>, message: String, #[serde(default)] seq: u64 },
}

impl ServerMsg {
    /// The request seq this message replies to; 0 for async messages and for
    /// frames from a seq-less v1 peer.
    pub fn seq(&self) -> u64 {
        match self {
            ServerMsg::Hello { seq, .. }
            | ServerMsg::Started { seq, .. }
            | ServerMsg::Captured { seq, .. }
            | ServerMsg::Status { seq, .. }
            | ServerMsg::List { seq, .. }
            | ServerMsg::Error { seq, .. } => *seq,
            ServerMsg::Exited { .. } => 0,
        }
    }
}

#[derive(Debug)]
pub enum ClientFrame {
    Msg(ClientMsg),
    Input { id: String, bytes: Vec<u8> },
}

#[derive(Debug)]
pub enum ServerFrame {
    Msg(ServerMsg),
    Output { id: String, bytes: Vec<u8> },
    Snapshot { id: String, cols: u16, rows: u16, cx: u16, cy: u16, data: Vec<u8> },
}

const T_JSON: u8 = 0;
const T_OUTPUT: u8 = 1;
const T_INPUT: u8 = 2;
const T_SNAPSHOT: u8 = 3;

pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    let len = payload.len() as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut lenb = [0u8; 4];
    r.read_exact(&mut lenb)?;
    let len = u32::from_le_bytes(lenb) as usize;
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok(payload)
}

pub fn encode_json<M: Serialize>(m: &M) -> Vec<u8> {
    let mut out = vec![T_JSON];
    out.extend_from_slice(&serde_json::to_vec(m).expect("serialize"));
    out
}

fn encode_id_bytes(t: u8, id: &str, bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![t];
    out.extend_from_slice(&(id.len() as u32).to_le_bytes());
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(bytes);
    out
}

pub fn encode_output(id: &str, bytes: &[u8]) -> Vec<u8> { encode_id_bytes(T_OUTPUT, id, bytes) }
pub fn encode_input(id: &str, bytes: &[u8]) -> Vec<u8> { encode_id_bytes(T_INPUT, id, bytes) }

pub fn encode_snapshot(id: &str, cols: u16, rows: u16, cx: u16, cy: u16, data: &[u8]) -> Vec<u8> {
    let mut out = vec![T_SNAPSHOT];
    out.extend_from_slice(&(id.len() as u32).to_le_bytes());
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(&cols.to_le_bytes());
    out.extend_from_slice(&rows.to_le_bytes());
    out.extend_from_slice(&cx.to_le_bytes());
    out.extend_from_slice(&cy.to_le_bytes());
    out.extend_from_slice(data);
    out
}

fn err(msg: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, msg) }

fn split_id(rest: &[u8]) -> io::Result<(String, &[u8])> {
    if rest.len() < 4 { return Err(err("short id frame")); }
    let idlen = u32::from_le_bytes(rest[0..4].try_into().unwrap()) as usize;
    if rest.len() < 4 + idlen { return Err(err("truncated id")); }
    let id = String::from_utf8_lossy(&rest[4..4 + idlen]).to_string();
    Ok((id, &rest[4 + idlen..]))
}

pub fn decode_client(payload: &[u8]) -> io::Result<ClientFrame> {
    match payload.first().copied() {
        Some(T_JSON) => Ok(ClientFrame::Msg(
            serde_json::from_slice(&payload[1..]).map_err(|e| err(&e.to_string()))?,
        )),
        Some(T_INPUT) => {
            let (id, bytes) = split_id(&payload[1..])?;
            Ok(ClientFrame::Input { id, bytes: bytes.to_vec() })
        }
        _ => Err(err("unknown client frame type")),
    }
}

pub fn decode_server(payload: &[u8]) -> io::Result<ServerFrame> {
    match payload.first().copied() {
        Some(T_JSON) => Ok(ServerFrame::Msg(
            serde_json::from_slice(&payload[1..]).map_err(|e| err(&e.to_string()))?,
        )),
        Some(T_OUTPUT) => {
            let (id, bytes) = split_id(&payload[1..])?;
            Ok(ServerFrame::Output { id, bytes: bytes.to_vec() })
        }
        Some(T_SNAPSHOT) => {
            let (id, rest) = split_id(&payload[1..])?;
            if rest.len() < 8 { return Err(err("short snapshot header")); }
            let cols = u16::from_le_bytes(rest[0..2].try_into().unwrap());
            let rows = u16::from_le_bytes(rest[2..4].try_into().unwrap());
            let cx = u16::from_le_bytes(rest[4..6].try_into().unwrap());
            let cy = u16::from_le_bytes(rest[6..8].try_into().unwrap());
            Ok(ServerFrame::Snapshot { id, cols, rows, cx, cy, data: rest[8..].to_vec() })
        }
        _ => Err(err("unknown server frame type")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_client_msg_round_trips() {
        let payload = encode_json(&ClientMsg::Capture { id: "a".into(), lines: 50, seq: 7 });
        match decode_client(&payload).unwrap() {
            ClientFrame::Msg(ClientMsg::Capture { id, lines, seq }) => {
                assert_eq!((id.as_str(), lines, seq), ("a", 50, 7));
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn input_frame_carries_raw_bytes() {
        let payload = encode_input("sess", &[0x00, 0xff, b'x']);
        match decode_client(&payload).unwrap() {
            ClientFrame::Input { id, bytes } => {
                assert_eq!(id, "sess");
                assert_eq!(bytes, vec![0x00, 0xff, b'x']);
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn snapshot_frame_round_trips() {
        let payload = encode_snapshot("s", 80, 24, 3, 4, b"\x1b[Hhi");
        match decode_server(&payload).unwrap() {
            ServerFrame::Snapshot { id, cols, rows, cx, cy, data } => {
                assert_eq!((id.as_str(), cols, rows, cx, cy), ("s", 80, 24, 3, 4));
                assert_eq!(data, b"\x1b[Hhi");
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn frame_length_prefix_round_trips() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &encode_output("id", b"abc")).unwrap();
        let mut cur = std::io::Cursor::new(buf);
        let payload = read_frame(&mut cur).unwrap();
        match decode_server(&payload).unwrap() {
            ServerFrame::Output { id, bytes } => {
                assert_eq!((id.as_str(), bytes.as_slice()), ("id", b"abc".as_slice()));
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn start_session_carries_fallback() {
        let msg = ClientMsg::StartSession {
            id: "r".into(), cwd: "/tmp".into(), command: "claude".into(),
            args: vec!["--continue".into()], env: vec![], cols: 80, rows: 24,
            fallback: Some(FallbackSpec {
                command: "claude".into(), args: vec![], grace_ms: 3000,
            }),
            seq: 1,
        };
        let payload = encode_json(&msg);
        match decode_client(&payload).unwrap() {
            ClientFrame::Msg(ClientMsg::StartSession { fallback: Some(fb), .. }) => {
                assert_eq!((fb.command.as_str(), fb.grace_ms), ("claude", 3000));
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn v1_reply_without_seq_decodes_as_seq_zero() {
        // A v1 daemon's Hello carries no seq; it must decode as seq 0 so the
        // client can recognize it as a legacy handshake.
        let json = br#"{"Hello":{"version":1}}"#;
        let mut payload = vec![0u8];
        payload.extend_from_slice(json);
        match decode_server(&payload).unwrap() {
            ServerFrame::Msg(ServerMsg::Hello { version, seq }) => {
                assert_eq!((version, seq), (1, 0));
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn start_session_fallback_defaults_to_none_when_absent() {
        // A JSON StartSession payload WITHOUT a `fallback` field must decode to None.
        let json = br#"{"StartSession":{"id":"r","cwd":"/tmp","command":"c","args":[],"env":[],"cols":80,"rows":24}}"#;
        let mut payload = vec![0u8]; // type 0 = JSON
        payload.extend_from_slice(json);
        match decode_client(&payload).unwrap() {
            ClientFrame::Msg(ClientMsg::StartSession { fallback: None, .. }) => {}
            other => panic!("expected fallback None, got {other:?}"),
        }
    }
}

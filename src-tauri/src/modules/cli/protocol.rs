//! Wire contract between `koden cli` (client) and the running Koden instance.
//!
//! One request per connection, both directions a single JSON line:
//!
//! ```text
//! -> {"token":"<hex32>","id":"<id>","cmd":"terminal.read","args":{...},"session":"12"}
//! <- {"id":"<id>","ok":true,"result":...}
//! <- {"id":"<id>","ok":false,"error":"..."}
//! ```
//!
//! The token never leaves the Rust side: the webview only ever sees the
//! [`Forwarded`] shape.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Hard cap on one request line and on one serialized reply.
pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
/// Requests parked on the webview at any one time.
pub const MAX_INFLIGHT: usize = 32;
/// How long a parked request waits for `cli_reply` before failing.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Tauri event the bridge listens to.
pub const REQUEST_EVENT: &str = "koden:cli-request";

pub const ENV_ENDPOINT: &str = "KODEN_CLI_ENDPOINT";
pub const ENV_TOKEN: &str = "KODEN_CLI_TOKEN";
pub const ENV_EXE: &str = "KODEN_EXE";
pub const ENV_SESSION: &str = "KODEN_SESSION";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Request {
    pub token: String,
    pub id: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub session: Option<String>,
}

/// What the webview receives: the request minus the token.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Forwarded {
    pub id: String,
    pub cmd: String,
    pub args: Value,
    pub session: Option<String>,
}

impl From<&Request> for Forwarded {
    fn from(r: &Request) -> Self {
        Self {
            id: r.id.clone(),
            cmd: r.cmd.clone(),
            args: r.args.clone(),
            session: r.session.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Reply {
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok(id: impl Into<String>, result: Value) -> Self {
        Self {
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    TooLarge(usize),
    BadJson(String),
    BadToken,
    Invalid(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(n) => write!(
                f,
                "request too large ({n} bytes; max {MAX_PAYLOAD_BYTES})"
            ),
            Self::BadJson(e) => write!(f, "request is not a JSON object: {e}"),
            Self::BadToken => write!(f, "invalid or missing token"),
            Self::Invalid(e) => write!(f, "malformed request: {e}"),
        }
    }
}

#[derive(Deserialize)]
struct TokenOnly {
    token: Option<String>,
}

/// Validates size and token BEFORE the rest of the envelope is deserialized:
/// an unauthenticated peer never gets to exercise the args parser.
pub fn parse_request(line: &str, expected_token: &str) -> Result<Request, ParseError> {
    if line.len() > MAX_PAYLOAD_BYTES {
        return Err(ParseError::TooLarge(line.len()));
    }
    let probe: TokenOnly =
        serde_json::from_str(line).map_err(|e| ParseError::BadJson(e.to_string()))?;
    if expected_token.is_empty() || probe.token.as_deref() != Some(expected_token) {
        return Err(ParseError::BadToken);
    }
    let req: Request =
        serde_json::from_str(line).map_err(|e| ParseError::Invalid(e.to_string()))?;
    if req.id.is_empty() || req.id.len() > 64 {
        return Err(ParseError::Invalid("id must be 1..64 chars".into()));
    }
    if req.cmd.is_empty() || req.cmd.len() > 64 {
        return Err(ParseError::Invalid("cmd must be 1..64 chars".into()));
    }
    if !req.args.is_object() {
        return Err(ParseError::Invalid("args must be an object".into()));
    }
    Ok(req)
}

pub fn encode_request(req: &Request) -> String {
    let mut s = serde_json::to_string(req).expect("request serializes");
    s.push('\n');
    s
}

/// Serialized reply line. Oversized results are replaced by an error reply so
/// the client never has to read past [`MAX_PAYLOAD_BYTES`].
pub fn encode_reply(reply: &Reply) -> String {
    let mut s = serde_json::to_string(reply).expect("reply serializes");
    if s.len() > MAX_PAYLOAD_BYTES {
        s = serde_json::to_string(&Reply::err(
            reply.id.clone(),
            format!(
                "reply too large ({} bytes; max {MAX_PAYLOAD_BYTES})",
                s.len()
            ),
        ))
        .expect("error reply serializes");
    }
    s.push('\n');
    s
}

pub fn decode_reply(line: &str) -> Result<Reply, String> {
    serde_json::from_str(line.trim_end()).map_err(|e| format!("malformed reply: {e}"))
}

/// Instance facts the Rust side folds into a successful `ping`.
#[derive(Clone, Debug)]
pub struct InstanceInfo {
    pub version: &'static str,
    pub pid: u32,
    pub endpoint: String,
}

/// `ping` is the one command whose answer is completed here: the webview
/// proves the bridge is alive, Rust adds what only it knows.
pub fn decorate_reply(cmd: &str, mut reply: Reply, info: &InstanceInfo) -> Reply {
    if cmd == "ping" && reply.ok {
        let mut obj = match reply.result.take() {
            Some(Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        obj.insert("version".into(), Value::String(info.version.to_string()));
        obj.insert("pid".into(), Value::from(info.pid));
        obj.insert("endpoint".into(), Value::String(info.endpoint.clone()));
        reply.result = Some(Value::Object(obj));
    }
    reply
}

/// 32 hex chars (128 bits). `RandomState` is seeded from OS randomness per
/// thread (std's hashmap_random_keys), so hashing under two independently
/// keyed hashers yields 128 unpredictable bits without a new crate. On Unix
/// /dev/urandom is preferred when readable.
pub fn generate_token() -> String {
    let bytes = os_random_16().unwrap_or_else(hash_random_16);
    let mut out = String::with_capacity(32);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Short per-request id: hex of process-local entropy plus a counter.
pub fn generate_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let salt = hash_random_16();
    format!(
        "{:08x}{:02x}{:02x}{:04x}",
        std::process::id(),
        salt[0],
        salt[1],
        n & 0xffff
    )
}

#[cfg(unix)]
fn os_random_16() -> Option<[u8; 16]> {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom").ok()?;
    let mut buf = [0u8; 16];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

#[cfg(not(unix))]
fn os_random_16() -> Option<[u8; 16]> {
    None
}

fn hash_random_16() -> [u8; 16] {
    let mut out = [0u8; 16];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stack_probe = &out as *const _ as usize;
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut h = RandomState::new().build_hasher();
        now.hash(&mut h);
        stack_probe.hash(&mut h);
        std::process::id().hash(&mut h);
        i.hash(&mut h);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn line(token: &str) -> String {
        format!(
            r#"{{"token":"{token}","id":"r1","cmd":"terminal.read","args":{{"lines":5}},"session":"12"}}"#
        )
    }

    #[test]
    fn round_trips_a_request() {
        let req = parse_request(&line(TOKEN), TOKEN).unwrap();
        assert_eq!(req.cmd, "terminal.read");
        assert_eq!(req.args["lines"], 5);
        assert_eq!(req.session.as_deref(), Some("12"));
        let encoded = encode_request(&req);
        assert!(encoded.ends_with('\n'));
        let again = parse_request(encoded.trim_end(), TOKEN).unwrap();
        assert_eq!(again, req);
    }

    #[test]
    fn rejects_bad_token_before_anything_else() {
        assert_eq!(
            parse_request(&line("nope"), TOKEN).unwrap_err(),
            ParseError::BadToken
        );
        // Missing token on an otherwise-broken envelope is still a token error.
        assert_eq!(
            parse_request(r#"{"id":5,"cmd":[]}"#, TOKEN).unwrap_err(),
            ParseError::BadToken
        );
        // An empty expected token never matches, even an empty client token.
        assert_eq!(
            parse_request(&line(""), "").unwrap_err(),
            ParseError::BadToken
        );
    }

    #[test]
    fn rejects_non_json_and_malformed_envelopes() {
        assert!(matches!(
            parse_request("not json", TOKEN).unwrap_err(),
            ParseError::BadJson(_)
        ));
        let no_id = format!(r#"{{"token":"{TOKEN}","cmd":"ping","args":{{}}}}"#);
        assert!(matches!(
            parse_request(&no_id, TOKEN).unwrap_err(),
            ParseError::Invalid(_)
        ));
        let bad_args = format!(r#"{{"token":"{TOKEN}","id":"a","cmd":"ping","args":[1]}}"#);
        assert!(matches!(
            parse_request(&bad_args, TOKEN).unwrap_err(),
            ParseError::Invalid(_)
        ));
    }

    #[test]
    fn args_and_session_default_when_omitted() {
        let l = format!(r#"{{"token":"{TOKEN}","id":"a","cmd":"ping"}}"#);
        let req = parse_request(&l, TOKEN).unwrap_err();
        // args defaults to Value::Null which is not an object: explicit contract.
        assert!(matches!(req, ParseError::Invalid(_)));
        let l = format!(r#"{{"token":"{TOKEN}","id":"a","cmd":"ping","args":{{}}}}"#);
        let req = parse_request(&l, TOKEN).unwrap();
        assert_eq!(req.session, None);
    }

    #[test]
    fn rejects_oversized_lines() {
        let big = format!(
            r#"{{"token":"{TOKEN}","id":"a","cmd":"ping","args":{{"x":"{}"}}}}"#,
            "y".repeat(MAX_PAYLOAD_BYTES)
        );
        assert!(matches!(
            parse_request(&big, TOKEN).unwrap_err(),
            ParseError::TooLarge(_)
        ));
    }

    #[test]
    fn reply_encoding_round_trips_and_caps_size() {
        let ok = Reply::ok("r1", serde_json::json!({"pong": true}));
        let s = encode_reply(&ok);
        assert!(s.ends_with('\n'));
        assert_eq!(decode_reply(&s).unwrap(), ok);
        assert!(!s.contains("error"));

        let err = Reply::err("r1", "boom");
        let s = encode_reply(&err);
        assert_eq!(decode_reply(&s).unwrap(), err);
        assert!(!s.contains("result"));

        let huge = Reply::ok("r2", Value::String("z".repeat(MAX_PAYLOAD_BYTES)));
        let s = encode_reply(&huge);
        let decoded = decode_reply(&s).unwrap();
        assert!(!decoded.ok);
        assert!(decoded.error.unwrap().contains("reply too large"));
        assert!(s.len() < 1024);
    }

    #[test]
    fn forwarded_strips_the_token() {
        let req = parse_request(&line(TOKEN), TOKEN).unwrap();
        let fwd = Forwarded::from(&req);
        let json = serde_json::to_string(&fwd).unwrap();
        assert!(!json.contains(TOKEN));
        assert!(json.contains(r#""cmd":"terminal.read""#));
    }

    #[test]
    fn ping_gets_instance_facts_and_other_cmds_do_not() {
        let info = InstanceInfo {
            version: "1.2.3",
            pid: 42,
            endpoint: "x".into(),
        };
        let r = decorate_reply("ping", Reply::ok("a", serde_json::json!({"pong": true})), &info);
        let res = r.result.unwrap();
        assert_eq!(res["pong"], true);
        assert_eq!(res["version"], "1.2.3");
        assert_eq!(res["pid"], 42);
        let r = decorate_reply(
            "terminal.list",
            Reply::ok("a", serde_json::json!({"count": 0})),
            &info,
        );
        assert!(r.result.unwrap().get("pid").is_none());
        let r = decorate_reply("ping", Reply::err("a", "no"), &info);
        assert!(r.result.is_none());
    }

    #[test]
    fn tokens_are_32_hex_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert_ne!(generate_id(), generate_id());
    }
}

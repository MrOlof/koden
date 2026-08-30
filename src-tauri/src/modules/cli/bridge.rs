//! Parks a socket connection until the webview answers via `cli_reply`.
//!
//! Transport-agnostic on purpose: `handle_connection` takes any
//! `Read + Write` stream and a `forward` closure, so the whole request
//! lifecycle (auth, cap, timeout, reply framing) is unit-tested in memory and
//! the platform listeners in `server.rs` stay a few lines each.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Mutex;
use std::time::Duration;

use super::protocol::{
    decorate_reply, encode_reply, parse_request, Forwarded, InstanceInfo, Reply, MAX_INFLIGHT,
    MAX_PAYLOAD_BYTES, REQUEST_TIMEOUT,
};

/// In-flight requests keyed by id. Bounded by [`MAX_INFLIGHT`].
#[derive(Default)]
pub struct Pending {
    slots: Mutex<HashMap<String, SyncSender<Reply>>>,
}

impl Pending {
    pub fn submit(&self, id: &str) -> Result<Receiver<Reply>, String> {
        let mut slots = self.slots.lock().expect("cli pending mutex poisoned");
        if slots.len() >= MAX_INFLIGHT {
            return Err(format!(
                "too many in-flight CLI requests ({MAX_INFLIGHT}); retry shortly"
            ));
        }
        if slots.contains_key(id) {
            return Err(format!("duplicate request id '{id}'"));
        }
        let (tx, rx) = mpsc::sync_channel(1);
        slots.insert(id.to_string(), tx);
        Ok(rx)
    }

    /// True when a parked connection was found and woken. Unknown ids (already
    /// timed out, or never issued) are ignored so a late reply is harmless.
    pub fn complete(&self, reply: Reply) -> bool {
        let tx = self
            .slots
            .lock()
            .expect("cli pending mutex poisoned")
            .remove(&reply.id);
        match tx {
            Some(tx) => tx.try_send(reply).is_ok(),
            None => false,
        }
    }

    pub fn remove(&self, id: &str) {
        self.slots
            .lock()
            .expect("cli pending mutex poisoned")
            .remove(id);
    }

    pub fn len(&self) -> usize {
        self.slots.lock().expect("cli pending mutex poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Everything a connection needs besides the stream itself.
pub struct Endpoint<'a> {
    pub token: &'a str,
    pub pending: &'a Pending,
    pub info: &'a InstanceInfo,
    /// Hands the request to the webview. `Err` means it could not be delivered.
    pub forward: &'a (dyn Fn(Forwarded) -> Result<(), String> + Sync),
    pub timeout: Duration,
}

/// Reads one request line (bounded), answers exactly one reply line.
pub fn handle_connection<S: Read + Write>(stream: &mut S, ep: &Endpoint<'_>) {
    let reply = match read_request_line(stream) {
        Ok(line) => process_line(&line, ep),
        Err(e) => Reply::err("", e),
    };
    let _ = stream.write_all(encode_reply(&reply).as_bytes());
    let _ = stream.flush();
}

/// Reads up to and including the first newline, refusing anything longer than
/// the payload cap so a peer cannot make the server buffer unbounded input.
fn read_request_line<S: Read>(stream: &mut S) -> Result<String, String> {
    let mut reader = BufReader::new(stream.take(MAX_PAYLOAD_BYTES as u64 + 1));
    let mut buf = Vec::with_capacity(512);
    reader
        .read_until(b'\n', &mut buf)
        .map_err(|e| format!("read failed: {e}"))?;
    if buf.len() > MAX_PAYLOAD_BYTES {
        return Err(format!(
            "request too large (> {MAX_PAYLOAD_BYTES} bytes)"
        ));
    }
    if buf.is_empty() {
        return Err("empty request".into());
    }
    String::from_utf8(buf).map_err(|_| "request is not UTF-8".to_string())
}

fn process_line(line: &str, ep: &Endpoint<'_>) -> Reply {
    let req = match parse_request(line.trim_end_matches(['\r', '\n']), ep.token) {
        Ok(r) => r,
        Err(e) => return Reply::err("", e.to_string()),
    };
    let rx = match ep.pending.submit(&req.id) {
        Ok(rx) => rx,
        Err(e) => return Reply::err(req.id, e),
    };
    if let Err(e) = (ep.forward)(Forwarded::from(&req)) {
        ep.pending.remove(&req.id);
        return Reply::err(req.id, format!("Koden did not accept the request: {e}"));
    }
    match rx.recv_timeout(ep.timeout) {
        Ok(reply) => decorate_reply(&req.cmd, reply, ep.info),
        Err(RecvTimeoutError::Timeout) => {
            ep.pending.remove(&req.id);
            Reply::err(
                req.id,
                format!(
                    "timed out after {} s waiting for Koden to answer '{}'",
                    ep.timeout.as_secs(),
                    req.cmd
                ),
            )
        }
        Err(RecvTimeoutError::Disconnected) => {
            ep.pending.remove(&req.id);
            Reply::err(req.id, "Koden dropped the request")
        }
    }
}

/// Default timeout for the real server; tests shorten it.
pub fn default_timeout() -> Duration {
    REQUEST_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Mutex as StdMutex;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn info() -> InstanceInfo {
        InstanceInfo {
            version: "0.0.0-test",
            pid: 7,
            endpoint: "mem".into(),
        }
    }

    /// In-memory duplex: the request is pre-loaded, the reply lands in `out`.
    struct MemStream {
        input: Cursor<Vec<u8>>,
        out: Vec<u8>,
    }

    impl MemStream {
        fn new(input: impl Into<Vec<u8>>) -> Self {
            Self {
                input: Cursor::new(input.into()),
                out: Vec::new(),
            }
        }
        fn reply(&self) -> Reply {
            super::super::protocol::decode_reply(std::str::from_utf8(&self.out).unwrap()).unwrap()
        }
    }

    impl Read for MemStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for MemStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.out.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn request(id: &str, cmd: &str) -> String {
        format!(r#"{{"token":"{TOKEN}","id":"{id}","cmd":"{cmd}","args":{{}},"session":"3"}}"#)
            + "\n"
    }

    #[test]
    fn round_trip_through_a_synchronous_webview() {
        let pending = Pending::default();
        let seen: StdMutex<Vec<Forwarded>> = StdMutex::new(Vec::new());
        let forward = |f: Forwarded| {
            seen.lock().unwrap().push(f.clone());
            // The "webview" answers synchronously.
            assert!(pending.complete(Reply::ok(f.id, serde_json::json!({"pong": true}))));
            Ok(())
        };
        let ep = Endpoint {
            token: TOKEN,
            pending: &pending,
            info: &info(),
            forward: &forward,
            timeout: Duration::from_secs(1),
        };
        let mut s = MemStream::new(request("r1", "ping"));
        handle_connection(&mut s, &ep);
        let reply = s.reply();
        assert!(reply.ok);
        assert_eq!(reply.id, "r1");
        let res = reply.result.unwrap();
        assert_eq!(res["pong"], true);
        assert_eq!(res["pid"], 7);
        assert_eq!(res["version"], "0.0.0-test");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].session.as_deref(), Some("3"));
        assert!(pending.is_empty());
    }

    #[test]
    fn bad_token_never_reaches_the_webview() {
        let pending = Pending::default();
        let forward = |_: Forwarded| -> Result<(), String> { panic!("must not forward") };
        let ep = Endpoint {
            token: TOKEN,
            pending: &pending,
            info: &info(),
            forward: &forward,
            timeout: Duration::from_millis(50),
        };
        let line = r#"{"token":"wrong","id":"r1","cmd":"terminal.read","args":{}}"#.to_string() + "\n";
        let mut s = MemStream::new(line);
        handle_connection(&mut s, &ep);
        let reply = s.reply();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("token"));
        assert!(pending.is_empty());
    }

    #[test]
    fn times_out_and_releases_the_slot() {
        let pending = Pending::default();
        let forward = |_: Forwarded| Ok(());
        let ep = Endpoint {
            token: TOKEN,
            pending: &pending,
            info: &info(),
            forward: &forward,
            timeout: Duration::from_millis(30),
        };
        let mut s = MemStream::new(request("slow", "terminal.list"));
        handle_connection(&mut s, &ep);
        let reply = s.reply();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("timed out"));
        assert!(pending.is_empty());
        // A late reply is ignored, not an error.
        assert!(!pending.complete(Reply::ok("slow", serde_json::json!({}))));
    }

    #[test]
    fn forward_failure_is_reported_and_releases_the_slot() {
        let pending = Pending::default();
        let forward = |_: Forwarded| Err("no window".to_string());
        let ep = Endpoint {
            token: TOKEN,
            pending: &pending,
            info: &info(),
            forward: &forward,
            timeout: Duration::from_millis(30),
        };
        let mut s = MemStream::new(request("x", "ping"));
        handle_connection(&mut s, &ep);
        let reply = s.reply();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("no window"));
        assert!(pending.is_empty());
    }

    #[test]
    fn in_flight_cap_is_enforced() {
        let pending = Pending::default();
        let mut held = Vec::new();
        for i in 0..MAX_INFLIGHT {
            held.push(pending.submit(&format!("id{i}")).unwrap());
        }
        let err = pending.submit("overflow").unwrap_err();
        assert!(err.contains("in-flight"));
        assert!(pending.submit("id0").is_err(), "duplicate ids are refused");
        pending.remove("id0");
        assert!(pending.submit("overflow").is_ok());
        drop(held);
    }

    #[test]
    fn oversized_and_empty_requests_are_refused_without_forwarding() {
        let pending = Pending::default();
        let forward = |_: Forwarded| -> Result<(), String> { panic!("must not forward") };
        let ep = Endpoint {
            token: TOKEN,
            pending: &pending,
            info: &info(),
            forward: &forward,
            timeout: Duration::from_millis(30),
        };
        let mut big = MemStream::new("x".repeat(MAX_PAYLOAD_BYTES + 5));
        handle_connection(&mut big, &ep);
        assert!(big.reply().error.unwrap().contains("too large"));

        let mut empty = MemStream::new("");
        handle_connection(&mut empty, &ep);
        assert!(empty.reply().error.unwrap().contains("empty"));
    }

    #[test]
    fn async_completion_from_another_thread() {
        let pending = std::sync::Arc::new(Pending::default());
        let p2 = pending.clone();
        let forward = move |f: Forwarded| {
            let p = p2.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(10));
                p.complete(Reply::ok(f.id, serde_json::json!({"count": 2})));
            });
            Ok(())
        };
        let ep = Endpoint {
            token: TOKEN,
            pending: &pending,
            info: &info(),
            forward: &forward,
            timeout: Duration::from_secs(2),
        };
        let mut s = MemStream::new(request("t", "terminal.list"));
        handle_connection(&mut s, &ep);
        let reply = s.reply();
        assert!(reply.ok);
        assert_eq!(reply.result.unwrap()["count"], 2);
    }
}

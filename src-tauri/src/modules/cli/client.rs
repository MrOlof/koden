//! `koden cli ...`: parse argv, talk to the owning Koden instance, print.
//!
//! Runs before any Tauri setup (see `main.rs`), so it must stay free of
//! app state and return an exit code instead of exiting itself.

use super::args::{parse, Invocation, Parsed};
use super::protocol::{
    decode_reply, encode_request, generate_id, Request, ENV_ENDPOINT, ENV_SESSION, ENV_TOKEN,
};
use super::render::render;

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

pub fn run(argv: Vec<String>) -> i32 {
    #[cfg(windows)]
    super::win::ensure_console();
    match parse(&argv) {
        Parsed::Help(text) => {
            println!("{text}");
            EXIT_OK
        }
        Parsed::Usage(msg) => {
            eprintln!("koden: {msg}\nRun 'koden --help' for usage.");
            EXIT_USAGE
        }
        Parsed::Invoke(inv) => execute(inv),
    }
}

fn execute(inv: Invocation) -> i32 {
    let endpoint = std::env::var(ENV_ENDPOINT).ok().filter(|s| !s.is_empty());
    let token = std::env::var(ENV_TOKEN).ok().filter(|s| !s.is_empty());
    let (Some(endpoint), Some(token)) = (endpoint, token) else {
        eprintln!(
            "koden: not running inside a Koden terminal ({ENV_ENDPOINT} / {ENV_TOKEN} are unset).\n\
             Open a terminal tab in Koden and run this command there."
        );
        return EXIT_ERROR;
    };
    let session = std::env::var(ENV_SESSION).ok().filter(|s| !s.is_empty());
    let req = Request {
        token,
        id: generate_id(),
        cmd: inv.cmd.clone(),
        args: serde_json::Value::Object(inv.args),
        session,
    };
    let line = match transport::roundtrip(&endpoint, &encode_request(&req)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "koden: could not reach the Koden instance at {endpoint}: {e}\n\
                 Is the Koden window that opened this terminal still running?"
            );
            return EXIT_ERROR;
        }
    };
    let reply = match decode_reply(&line) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("koden: {e}");
            return EXIT_ERROR;
        }
    };
    if inv.json {
        println!("{}", line.trim_end());
    }
    if !reply.ok {
        eprintln!(
            "koden: {}",
            reply.error.unwrap_or_else(|| "unknown error".into())
        );
        return EXIT_ERROR;
    }
    if !inv.json {
        let text = render(&inv.cmd, reply.result.as_ref());
        if !text.is_empty() {
            println!("{text}");
        }
    }
    EXIT_OK
}

/// Connect, write one request line, read one reply line.
pub mod transport {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::time::Duration;

    use super::super::protocol::MAX_PAYLOAD_BYTES;

    const CONNECT_RETRIES: u32 = 50;
    const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(40);

    pub fn roundtrip(endpoint: &str, request_line: &str) -> Result<String, String> {
        let mut stream = connect(endpoint)?;
        stream
            .write_all(request_line.as_bytes())
            .map_err(|e| format!("write: {e}"))?;
        stream.flush().map_err(|e| format!("flush: {e}"))?;
        half_close(&stream);
        let mut reader = BufReader::new((&mut stream).take(MAX_PAYLOAD_BYTES as u64 + 1));
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("read: {e}"))?;
        if line.is_empty() {
            return Err("connection closed without a reply".into());
        }
        if line.len() > MAX_PAYLOAD_BYTES {
            return Err("reply too large".into());
        }
        Ok(line)
    }

    #[cfg(unix)]
    type Stream = std::os::unix::net::UnixStream;
    #[cfg(windows)]
    type Stream = std::fs::File;

    #[cfg(unix)]
    fn connect(endpoint: &str) -> Result<Stream, String> {
        let s = Stream::connect(endpoint).map_err(|e| e.to_string())?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(45)));
        Ok(s)
    }

    #[cfg(unix)]
    fn half_close(s: &Stream) {
        let _ = s.shutdown(std::net::Shutdown::Write);
    }

    /// The server hands out one instance per client; between an accept and the
    /// next `CreateNamedPipe` the name reports busy, so retry briefly.
    #[cfg(windows)]
    fn connect(endpoint: &str) -> Result<Stream, String> {
        use super::super::win::ERROR_PIPE_BUSY;
        let mut last = None;
        for _ in 0..CONNECT_RETRIES {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(endpoint)
            {
                Ok(f) => return Ok(f),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    last = Some(e);
                    std::thread::sleep(CONNECT_RETRY_DELAY);
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Err(last
            .map(|e| e.to_string())
            .unwrap_or_else(|| "pipe busy".into()))
    }

    // Named pipes have no half-close; the server reads up to the newline.
    #[cfg(windows)]
    fn half_close(_s: &Stream) {}
}

//! Local listener: a Unix socket (0600) or a Windows named pipe, one thread per
//! connection, each connection one request. The listener lives for the
//! process; the OS reclaims it on exit.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::bridge::{default_timeout, handle_connection, Endpoint, Pending};
use super::protocol::{encode_reply, Forwarded, InstanceInfo, Reply};

/// Connection threads alive at once (beyond the in-flight cap on requests, so
/// idle peers cannot pin threads forever).
const MAX_CONNECTIONS: usize = 64;
/// A peer that connects and never sends is dropped after this.
#[cfg(unix)]
const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub type Forward = Box<dyn Fn(Forwarded) -> Result<(), String> + Send + Sync>;

pub struct Shared {
    pub token: String,
    pub pending: Arc<Pending>,
    pub info: InstanceInfo,
    pub forward: Forward,
    pub timeout: Duration,
    active: AtomicUsize,
}

impl Shared {
    fn endpoint(&self) -> Endpoint<'_> {
        Endpoint {
            token: &self.token,
            pending: &self.pending,
            info: &self.info,
            forward: &*self.forward,
            timeout: self.timeout,
        }
    }
}

/// Default endpoint for this process.
pub fn default_endpoint() -> String {
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\koden-{}", std::process::id())
    }
    #[cfg(unix)]
    {
        unix::socket_dir()
            .join(format!("koden-{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }
}

/// Starts listening at the process default endpoint.
pub fn start(token: String, pending: Arc<Pending>, forward: Forward) -> Result<String, String> {
    start_at(default_endpoint(), token, pending, forward, default_timeout())
}

/// Starts listening at an explicit endpoint (tests use unique names so several
/// servers can coexist in one process).
pub fn start_at(
    endpoint: String,
    token: String,
    pending: Arc<Pending>,
    forward: Forward,
    timeout: Duration,
) -> Result<String, String> {
    let shared = Arc::new(Shared {
        token,
        pending,
        info: InstanceInfo {
            version: env!("CARGO_PKG_VERSION"),
            pid: std::process::id(),
            endpoint: endpoint.clone(),
        },
        forward,
        timeout,
        active: AtomicUsize::new(0),
    });
    #[cfg(unix)]
    unix::listen(&endpoint, shared)?;
    #[cfg(windows)]
    windows::listen(&endpoint, shared)?;
    Ok(endpoint)
}

/// Runs one connection on its own thread, or refuses it when the thread cap
/// is reached (the refusal is itself a well-formed reply line).
fn serve<S>(shared: Arc<Shared>, mut stream: S, finish: impl FnOnce(&mut S) + Send + 'static)
where
    S: std::io::Read + std::io::Write + Send + 'static,
{
    if shared.active.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
        shared.active.fetch_sub(1, Ordering::AcqRel);
        let _ = stream.write_all(
            encode_reply(&Reply::err(
                "",
                format!("too many CLI connections ({MAX_CONNECTIONS}); retry shortly"),
            ))
            .as_bytes(),
        );
        let _ = stream.flush();
        finish(&mut stream);
        return;
    }
    let spawned = thread::Builder::new()
        .name("koden-cli-conn".into())
        .spawn(move || {
            handle_connection(&mut stream, &shared.endpoint());
            finish(&mut stream);
            shared.active.fetch_sub(1, Ordering::AcqRel);
        });
    if spawned.is_err() {
        log::warn!("cli: could not spawn connection thread");
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

    pub fn socket_dir() -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
        {
            return dir;
        }
        if let Some(dir) = dirs::runtime_dir().filter(|p| p.is_dir()) {
            return dir;
        }
        if let Some(cache) = dirs::cache_dir() {
            let dir = cache.join("koden");
            if std::fs::create_dir_all(&dir).is_ok() {
                return dir;
            }
        }
        std::env::temp_dir()
    }

    pub fn listen(path: &str, shared: Arc<Shared>) -> Result<(), String> {
        let p = PathBuf::from(path);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("remove stale {path}: {e}"))?;
        }
        let listener = UnixListener::bind(&p).map_err(|e| format!("bind {path}: {e}"))?;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("chmod {path}: {e}"))?;
        thread::Builder::new()
            .name("koden-cli-listen".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(s) => {
                            let _ = s.set_read_timeout(Some(READ_TIMEOUT));
                            serve(shared.clone(), s, |s: &mut UnixStream| {
                                let _ = s.shutdown(std::net::Shutdown::Both);
                            });
                        }
                        Err(e) => log::debug!("cli: accept failed: {e}"),
                    }
                }
            })
            .map_err(|e| format!("spawn listener: {e}"))?;
        Ok(())
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::modules::cli::win;
    use std::fs::File;

    pub fn listen(name: &str, shared: Arc<Shared>) -> Result<(), String> {
        let first = win::create_instance(name, true).map_err(|e| format!("create {name}: {e}"))?;
        let name = name.to_string();
        thread::Builder::new()
            .name("koden-cli-listen".into())
            .spawn(move || {
                let mut instance = first;
                loop {
                    match win::accept(instance) {
                        Ok(file) => serve(shared.clone(), file, |f: &mut File| {
                            // Wait for the client to drain the reply before the
                            // handle drops; a disconnect discards unread bytes.
                            let _ = f.sync_all();
                        }),
                        Err(e) => log::debug!("cli: pipe accept failed: {e}"),
                    }
                    instance = match win::create_instance(&name, false) {
                        Ok(h) => h,
                        Err(e) => {
                            log::warn!("cli: could not create pipe instance: {e}");
                            thread::sleep(Duration::from_millis(200));
                            match win::create_instance(&name, false) {
                                Ok(h) => h,
                                Err(e) => {
                                    log::error!("cli: listener stopped: {e}");
                                    return;
                                }
                            }
                        }
                    };
                }
            })
            .map_err(|e| format!("spawn listener: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::cli::client::transport;
    use crate::modules::cli::protocol::{decode_reply, encode_request, Request};
    use std::sync::atomic::AtomicUsize;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    fn unique_endpoint() -> String {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let base = default_endpoint();
        #[cfg(windows)]
        {
            format!("{base}-t{n}")
        }
        #[cfg(unix)]
        {
            base.replace(".sock", &format!("-t{n}.sock"))
        }
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn request(token: &str, cmd: &str) -> String {
        encode_request(&Request {
            token: token.into(),
            id: "loop1".into(),
            cmd: cmd.into(),
            args: serde_json::json!({}),
            session: Some("9".into()),
        })
    }

    /// Real transport: server thread on the pipe/socket, client on the other
    /// end, a fake "webview" completing the parked request from a thread.
    #[test]
    fn loopback_round_trip_over_the_real_transport() {
        let pending = Arc::new(Pending::default());
        let p2 = pending.clone();
        let forward: Forward = Box::new(move |f: Forwarded| {
            assert_eq!(f.session.as_deref(), Some("9"));
            let p = p2.clone();
            thread::spawn(move || {
                p.complete(Reply::ok(f.id, serde_json::json!({"pong": true})));
            });
            Ok(())
        });
        let endpoint = start_at(
            unique_endpoint(),
            TOKEN.into(),
            pending.clone(),
            forward,
            Duration::from_secs(5),
        )
        .expect("server starts");

        let line = transport::roundtrip(&endpoint, &request(TOKEN, "ping")).expect("roundtrip");
        let reply = decode_reply(&line).unwrap();
        assert!(reply.ok, "{reply:?}");
        let res = reply.result.unwrap();
        assert_eq!(res["pong"], true);
        assert_eq!(res["pid"], std::process::id());
        assert_eq!(res["endpoint"], endpoint);

        // Second connection on the same listener (the pipe re-creates instances).
        let line = transport::roundtrip(&endpoint, &request("bad", "ping")).expect("roundtrip");
        let reply = decode_reply(&line).unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("token"));
        assert!(pending.is_empty());
    }

    #[test]
    fn loopback_timeout_reaches_the_client() {
        let pending = Arc::new(Pending::default());
        let forward: Forward = Box::new(|_| Ok(()));
        let endpoint = start_at(
            unique_endpoint(),
            TOKEN.into(),
            pending.clone(),
            forward,
            Duration::from_millis(50),
        )
        .expect("server starts");
        let line = transport::roundtrip(&endpoint, &request(TOKEN, "terminal.list")).unwrap();
        let reply = decode_reply(&line).unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("timed out"));
        assert!(pending.is_empty());
    }

    #[test]
    fn concurrent_clients_are_all_answered() {
        let pending = Arc::new(Pending::default());
        let p2 = pending.clone();
        let forward: Forward = Box::new(move |f: Forwarded| {
            let p = p2.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                p.complete(Reply::ok(f.id, serde_json::json!({"cmd": f.cmd})));
            });
            Ok(())
        });
        let endpoint = start_at(
            unique_endpoint(),
            TOKEN.into(),
            pending.clone(),
            forward,
            Duration::from_secs(5),
        )
        .expect("server starts");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let ep = endpoint.clone();
                thread::spawn(move || {
                    let req = encode_request(&Request {
                        token: TOKEN.into(),
                        id: format!("c{i}"),
                        cmd: format!("cmd{i}"),
                        args: serde_json::json!({}),
                        session: None,
                    });
                    let line = transport::roundtrip(&ep, &req).expect("roundtrip");
                    let reply = decode_reply(&line).unwrap();
                    assert!(reply.ok, "{reply:?}");
                    assert_eq!(reply.id, format!("c{i}"));
                    assert_eq!(reply.result.unwrap()["cmd"], format!("cmd{i}"));
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert!(pending.is_empty());
    }
}

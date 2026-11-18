//! Opening a login link while SuperChat is already running: the second process hands the link to the first one over a
//! loopback socket and exits. The socket's port and a random secret live in a private file in the data directory, so
//! only processes of the same user can use it (a web page cannot guess the secret).

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const FILE: &str = "instance.sock";
const OK: &[u8] = b"SUPERCHAT-OK\n";

fn secret() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u8 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u8(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out += &format!("{:016x}", h.finish());
    }
    out
}

fn file(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Gives `link` to an already running instance. True when one took it (this process should exit).
pub fn send(dir: &Path, link: &str) -> bool {
    let Ok(txt) = std::fs::read_to_string(file(dir)) else { return false };
    let mut it = txt.lines();
    let (Some(port), Some(sec)) = (it.next().and_then(|p| p.parse::<u16>().ok()), it.next()) else { return false };
    let Ok(mut s) = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_millis(500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if s.write_all(format!("{sec}\n{link}\n").as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 32];
    matches!(s.read(&mut buf), Ok(n) if &buf[..n] == OK)
}

/// Starts listening for links from later launches. Received links are stored in `slot`.
pub fn listen(dir: &Path, slot: Arc<Mutex<Option<String>>>) {
    let Ok(l) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else { return };
    let Ok(addr) = l.local_addr() else { return };
    let sec = secret();
    let f = file(dir);
    let tmp = f.with_extension("tmp");
    if std::fs::write(&tmp, format!("{}\n{sec}\n", addr.port())).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    if std::fs::rename(&tmp, &f).is_err() {
        return;
    }
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut conn = conn;
            let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
            let mut data = Vec::new();
            let _ = Read::by_ref(&mut conn).take(8192).read_until_newlines(&mut data, 2);
            let text = String::from_utf8_lossy(&data).to_string();
            let mut lines = text.lines();
            if lines.next() == Some(sec.as_str()) {
                if let Some(link) = lines.next().filter(|l| proto::rules::extract_login_link(l).is_some()) {
                    if let Ok(mut g) = slot.lock() {
                        *g = Some(link.to_string());
                    }
                    let _ = conn.write_all(OK);
                }
            }
        }
    });
}

trait ReadLines {
    fn read_until_newlines(&mut self, out: &mut Vec<u8>, n: usize) -> std::io::Result<()>;
}

impl<R: Read> ReadLines for R {
    fn read_until_newlines(&mut self, out: &mut Vec<u8>, n: usize) -> std::io::Result<()> {
        let mut b = [0u8; 1];
        let mut seen = 0;
        while seen < n && self.read(&mut b)? == 1 {
            out.push(b[0]);
            if b[0] == b'\n' {
                seen += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_launch_hands_the_link_over() {
        let d = std::env::temp_dir().join(format!("sc-handoff-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let tok = "abcdefghijklmnopqrstuvwxyz012345";
        let link = format!("superchat://login?server=https%3A%2F%2Fx.example&token={tok}");
        assert!(!send(&d, &link), "nobody is listening yet");
        let slot = Arc::new(Mutex::new(None));
        listen(&d, slot.clone());
        assert!(send(&d, &link));
        for _ in 0..50 {
            if slot.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(slot.lock().unwrap().take().as_deref(), Some(link.as_str()));
        assert!(!send(&d, "https://x.example/not-a-login-link"), "only login links are accepted");
        // A wrong secret is refused.
        let port: u16 = std::fs::read_to_string(file(&d)).unwrap().lines().next().unwrap().parse().unwrap();
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(format!("wrong\n{link}\n").as_bytes()).unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(s.read(&mut buf).unwrap_or(0), 0);
        assert!(slot.lock().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}

#![allow(dead_code)]
use atomic_server::config::Config;
use client_core::admin::AdminClient;
use client_core::{Client, ClientConfig, ClientEvent};
use proto::*;
use std::path::PathBuf;
use std::time::Duration;

pub struct TestServer {
    pub url: String,
    pub dir: tempfile::TempDir,
    pub st: atomic_server::state::St,
    pub admin: AdminClient,
}

impl TestServer {
    /// Makes every unused login link expired.
    pub fn expire_login_links(&self) {
        let st = self.st.clone();
        let rt = tokio::runtime::Handle::current();
        std::thread::scope(|sc| {
            sc.spawn(|| rt.block_on(async { st.db.call(|c| { c.execute("UPDATE login_links SET expires_ms=1", [])?; Ok(()) }).await.unwrap() })).join().unwrap();
        });
    }

    pub async fn start() -> TestServer {
        Self::start_with(|_| {}).await
    }

    pub async fn start_with(f: impl FnOnce(&mut Config)) -> TestServer {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::for_test(dir.path().join("server"));
        f(&mut cfg);
        let st = atomic_server::build_state(cfg).await.unwrap();
        let url = atomic_server::serve_for_test(st.clone()).await.unwrap();
        let admin = AdminClient::setup(
            &url,
            &SetupRequest { username: "admin".into(), display_name: "Admin".into(), password: "test-admin-pw".into(), email: Some("admin@example.com".into()), storage: None },
        )
        .await
        .unwrap();
        TestServer { url, dir, st, admin }
    }

    pub fn client_dir(&self, name: &str) -> PathBuf {
        self.dir.path().join(format!("client-{name}"))
    }

    pub fn new_client(&self, name: &str) -> Client {
        let mut cfg = ClientConfig::new(self.client_dir(name));
        cfg.allow_insecure_http = true;
        cfg.device_name = format!("test-{name}");
        Client::open(cfg).unwrap()
    }

    /// Creates a user with a password and returns (user id, signed-in started client).
    pub async fn user(&self, name: &str) -> (String, Client) {
        let r = self.admin.create_user(name, &capitalise(name), Some("password-123")).await.unwrap();
        let c = self.new_client(name);
        c.login(&self.url, name, "password-123", None).await.unwrap();
        c.start();
        wait_online(&c).await;
        (r.user.user.id, c)
    }
}

pub fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

pub async fn wait_until<F: FnMut() -> bool>(what: &str, mut f: F) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for: {what}");
}

pub async fn wait_online(c: &Client) {
    wait_until("client online", || c.connection() == client_core::ConnState::Online).await;
}

pub async fn wait_signed_out(c: &Client) -> (Option<ErrorCode>, String) {
    let mut rx = c.subscribe();
    for _ in 0..200 {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(ClientEvent::SignedOut { code, message })) => return (code, message),
            Ok(_) => {}
            Err(_) => {
                if !c.signed_in() {
                    return (None, String::new());
                }
            }
        }
    }
    panic!("client was never signed out");
}

pub fn texts(c: &Client, conv: &str) -> Vec<String> {
    c.timeline(conv, 200, None).into_iter().map(|t| t.message.body).collect()
}

/// Waits until a message with this body is in the (top-level) timeline and returns it.
pub async fn wait_msg(c: &Client, conv: &str, body: &str) -> Message {
    let b = body.to_string();
    wait_until(&format!("message {body:?} present"), || c.timeline(conv, 500, None).iter().any(|t| t.message.body == b && !t.local)).await;
    c.timeline(conv, 500, None).into_iter().find(|t| t.message.body == body && !t.local).unwrap().message
}

/// Sends text and returns the server copy once it has been acknowledged.
pub async fn say(c: &Client, conv: &str, body: &str) -> Message {
    c.send_text(conv, body, client_core::SendOptions::default()).unwrap();
    wait_msg(c, conv, body).await
}

pub async fn make_channel(s: &TestServer, name: &str, members: &[&String]) -> String {
    let ids: Vec<String> = members.iter().map(|m| m.to_string()).collect();
    s.admin.create_channel(name, &ids).await.unwrap().conversation.id
}

pub async fn wait_conv(c: &Client, conv: &str) {
    wait_until("conversation arrives", || c.conversations().iter().any(|x| x.conversation.id == conv)).await;
}

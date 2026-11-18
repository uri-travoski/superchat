//! SV-13: wake-up pushes through fake ntfy / FCM / APNs servers. Nothing here talks to the real
//! services. Pushes carry no message content and respect online state, DND, mute and notify level.
mod common;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, Uri};
use axum::routing::any;
use axum::Router;
use client_core::Client;
use common::*;
use proto::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

#[derive(Clone, Default)]
struct Fake {
    reqs: Arc<Mutex<Vec<Req>>>,
}

impl Fake {
    async fn start() -> (Fake, String) {
        let f = Fake::default();
        let f2 = f.clone();
        let app = Router::new().fallback(any(move |m: Method, u: Uri, h: HeaderMap, b: Bytes| {
            let f = f2.clone();
            async move {
                let req = Req {
                    method: m.to_string(),
                    path: u.path().to_string(),
                    headers: h.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect(),
                    body: String::from_utf8_lossy(&b).to_string(),
                };
                let reply = if req.path.ends_with("/oauth2/token") { r#"{"access_token":"fake-access","expires_in":3600}"#.to_string() } else { "{}".to_string() };
                f.reqs.lock().unwrap().push(req);
                ([("content-type", "application/json")], reply)
            }
        }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (f, url)
    }

    /// Requests that are real pushes (not the OAuth token exchange).
    fn pushes(&self) -> Vec<Req> {
        self.reqs.lock().unwrap().iter().filter(|r| !r.path.ends_with("/oauth2/token")).cloned().collect()
    }
}

async fn offline(s: &TestServer, c: &Client, id: &str) {
    c.stop();
    wait_until("server sees user offline", || !s.st.hub.is_online(id)).await;
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(600)).await;
}

fn ntfy_reg(topic: &str) -> PushRegistration {
    PushRegistration { id: String::new(), kind: "ntfy".into(), target: topic.into(), ntfy_url: None, device_name: "phone".into(), created_ms: 0 }
}

#[tokio::test]
async fn ntfy_wake_only_when_offline_and_content_free() {
    let (fake, url) = Fake::start().await;
    let s = TestServer::start().await;
    s.admin.put_push_settings(&PushSettings { ntfy_url: Some(url.clone()), ..Default::default() }).await.unwrap();
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let dm = alice.open_dm(&b_id).await.unwrap();
    wait_conv(&bob, &dm).await;
    let ch = make_channel(&s, "team", &[&a_id, &b_id]).await;
    wait_conv(&bob, &ch).await;
    let reg = bob.register_push(&ntfy_reg("bob-topic")).await.unwrap();
    assert_eq!(bob.push_registrations().await.unwrap().len(), 1);

    // Bob has a live connection: no push.
    say(&alice, &dm, "secret plan A").await;
    settle().await;
    assert!(fake.pushes().is_empty(), "no push while the WebSocket is live");

    // Bob goes offline: one content-free wake-up.
    offline(&s, &bob, &b_id).await;
    say(&alice, &dm, "secret plan B").await;
    wait_until("ntfy wake", || fake.pushes().len() == 1).await;
    let p = fake.pushes().remove(0);
    assert_eq!((p.method.as_str(), p.path.as_str()), ("POST", "/bob-topic"));
    assert_eq!(p.body, "wake");
    let all = format!("{:?}", fake.reqs.lock().unwrap());
    for leak in ["secret", "plan", "Alice", "alice", &a_id, &dm] {
        assert!(!all.contains(leak), "push leaked {leak:?}: {all}");
    }

    // Do Not Disturb silences it.
    bob.set_presence_pref("dnd").await.unwrap_or_else(|_| panic!("set dnd"));
    say(&alice, &dm, "while dnd").await;
    settle().await;
    assert_eq!(fake.pushes().len(), 1);
    // (bob is signed in but offline over WS; the REST call above worked without a connection.)
    bob.set_presence_pref("auto").await.unwrap();

    // Muted conversation: silent.
    bob.mute(&dm, "forever", 0).await.unwrap();
    say(&alice, &dm, "while muted").await;
    settle().await;
    assert_eq!(fake.pushes().len(), 1);
    bob.mute(&dm, "off", 0).await.unwrap();

    // Notify level "mentions": plain messages are silent, mentions in channels are not.
    bob.set_pref(&ch, &ConvPrefRequest { notify: Some("mentions".into()), ..Default::default() }).await.unwrap();
    say(&alice, &ch, "plain chatter").await;
    settle().await;
    assert_eq!(fake.pushes().len(), 1);
    say(&alice, &ch, "@bob ping").await;
    wait_until("mention wake", || fake.pushes().len() == 2).await;
    bob.set_pref(&ch, &ConvPrefRequest { notify: Some("nothing".into()), ..Default::default() }).await.unwrap();
    say(&alice, &ch, "@bob ping again").await;
    settle().await;
    assert_eq!(fake.pushes().len(), 2);

    // Unregistering stops pushes.
    bob.unregister_push(&reg.id).await.unwrap();
    say(&alice, &dm, "after unregister").await;
    settle().await;
    assert_eq!(fake.pushes().len(), 2);
}

#[tokio::test]
async fn per_device_ntfy_server_and_revoked_session_gets_nothing() {
    let (fake_default, url_default) = Fake::start().await;
    let (fake_own, url_own) = Fake::start().await;
    let s = TestServer::start().await;
    s.admin.put_push_settings(&PushSettings { ntfy_url: Some(url_default), ..Default::default() }).await.unwrap();
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let dm = alice.open_dm(&b_id).await.unwrap();
    wait_conv(&bob, &dm).await;
    let mut reg = ntfy_reg("own-topic");
    reg.ntfy_url = Some(url_own);
    bob.register_push(&reg).await.unwrap();
    offline(&s, &bob, &b_id).await;
    say(&alice, &dm, "hello").await;
    wait_until("own ntfy server used", || fake_own.pushes().len() == 1).await;
    assert!(fake_default.pushes().is_empty());
    // Revoking bob's sessions removes his registrations from consideration.
    s.admin.revoke_user_sessions(&b_id).await.unwrap();
    say(&alice, &dm, "hello again").await;
    settle().await;
    assert_eq!(fake_own.pushes().len(), 1);
}

#[tokio::test]
async fn fcm_http_v1_and_apns_are_content_free() {
    let (fake, url) = Fake::start().await;
    let s = TestServer::start().await;
    let sa = serde_json::json!({
        "client_email": "svc@proj.iam.gserviceaccount.com",
        "private_key": include_str!("fixtures/test_rsa_pkcs8.pem"),
        "project_id": "proj-1"
    });
    s.admin
        .put_push_settings(&PushSettings {
            fcm_service_account: Some(sa.to_string()),
            fcm_api_base: Some(url.clone()),
            apns_key_pem: Some(include_str!("fixtures/test_ec_pkcs8.pem").to_string()),
            apns_key_id: Some("KEY123".into()),
            apns_team_id: Some("TEAM123".into()),
            apns_topic: Some("au.superchat".into()),
            apns_api_base: Some(url.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    // Reading the settings back never reveals secrets.
    let shown = s.admin.push_settings().await.unwrap();
    assert_eq!(shown.fcm_service_account.as_deref(), Some(PushSettings::MASK));
    assert_eq!(shown.apns_key_pem.as_deref(), Some(PushSettings::MASK));

    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let dm = alice.open_dm(&b_id).await.unwrap();
    wait_conv(&bob, &dm).await;
    for (kind, token) in [("fcm", "fcm-device-token"), ("apns", "apns-device-token")] {
        bob.register_push(&PushRegistration { id: String::new(), kind: kind.into(), target: token.into(), ntfy_url: None, device_name: kind.into(), created_ms: 0 }).await.unwrap();
    }
    offline(&s, &bob, &b_id).await;
    say(&alice, &dm, "top secret text").await;
    wait_until("both pushes", || fake.pushes().len() == 2).await;
    let pushes = fake.pushes();
    let fcm = pushes.iter().find(|p| p.path.contains("messages:send")).expect("fcm request");
    assert_eq!(fcm.path, "/v1/projects/proj-1/messages:send");
    assert!(fcm.headers.iter().any(|(k, v)| k == "authorization" && v == "Bearer fake-access"));
    let j: serde_json::Value = serde_json::from_str(&fcm.body).unwrap();
    assert_eq!(j["message"]["token"], "fcm-device-token");
    assert_eq!(j["message"]["data"]["type"], "wake");
    assert!(j["message"].get("notification").is_none(), "no notification payload: the app builds its own");
    // The OAuth exchange used a signed RS256 JWT bearer assertion.
    let tok = fake.reqs.lock().unwrap().iter().find(|r| r.path.ends_with("/oauth2/token")).cloned().expect("token request");
    assert!(tok.body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"));
    let assertion = tok.body.split("assertion=").nth(1).unwrap();
    assert_eq!(assertion.split('.').count(), 3);

    let apns = pushes.iter().find(|p| p.path.starts_with("/3/device/")).expect("apns request");
    assert_eq!(apns.path, "/3/device/apns-device-token");
    let hv = |n: &str| apns.headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap_or_default();
    assert!(hv("authorization").starts_with("bearer ") && hv("authorization").matches('.').count() == 2);
    assert_eq!(hv("apns-push-type"), "background");
    assert_eq!(hv("apns-topic"), "au.superchat");
    let j: serde_json::Value = serde_json::from_str(&apns.body).unwrap();
    assert_eq!(j["aps"]["content-available"], 1);
    assert!(j["aps"].get("alert").is_none());
    let all = format!("{:?}", fake.reqs.lock().unwrap());
    assert!(!all.contains("secret") && !all.contains("Alice"), "{all}");
}

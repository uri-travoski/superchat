//! SV-24, Phase 3: bots, webhooks, forms, scheduled messages and AI against the real server image.
//!
//! The server runs in docker with local storage; the receivers (bot webhook, outgoing webhook, fake AI
//! provider) run in this process and are reached through `host.docker.internal`. Run with `tests/e2e/run.sh`
//! (needs docker; skipped without ATOMIC_E2E).
//!
//! Covers: Telegram-style sendMessage / sendDocument / getUpdates / setWebhook delivery (secret header),
//! incoming webhook, outgoing webhook with a verified HMAC signature, a form submit round trip, a scheduled
//! message that survives a server restart, and AI search with a fake provider.
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::routing::any;
use axum::Router;
use client_core::admin::AdminClient;
use client_core::{Client, ClientConfig, ConnState, SendOptions};
use hmac::{Hmac, Mac};
use proto::*;
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PORT: u16 = 18302;
const IMAGE: &str = "atomic-chat-server:e2e";
const NAME: &str = "atomic-e2e-bots";

fn sh(args: &[&str]) -> (bool, String) {
    let o = Command::new("docker").args(args).output().expect("docker");
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = sh(&["rm", "-f", "-v", NAME]);
    }
}

async fn wait_for<F: FnMut() -> bool>(what: &str, mut f: F) {
    for _ in 0..600 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for: {what}");
}

async fn wait_health(url: &str) {
    let http = reqwest::Client::new();
    for _ in 0..200 {
        if let Ok(r) = http.get(format!("{url}/api/v1/health")).send().await {
            if r.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("server never became healthy");
}

#[derive(Clone, Default)]
struct Fake {
    reqs: Arc<Mutex<Vec<(String, HeaderMap, String)>>>,
}

impl Fake {
    async fn start() -> (Fake, u16) {
        let f = Fake::default();
        async fn handle(State(f): State<Fake>, u: Uri, h: HeaderMap, b: Bytes) -> axum::Json<Value> {
            let body = String::from_utf8_lossy(&b).to_string();
            f.reqs.lock().unwrap().push((u.path().to_string(), h, body.clone()));
            // Doubles as the fake AI provider (Anthropic messages API).
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let system = v["system"].as_str().unwrap_or("");
            let text = if system.contains("search filter") { r#"{"q": "invoice", "from": null, "in": null, "has": null, "after": null, "before": null}"# } else { "ok" };
            axum::Json(json!({"content": [{"type": "text", "text": text}], "role": "assistant"}))
        }
        let app = Router::new().fallback(any(handle)).with_state(f.clone());
        let l = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (f, port)
    }
    fn on(&self, path: &str) -> Vec<(HeaderMap, String)> {
        self.reqs.lock().unwrap().iter().filter(|r| r.0 == path).map(|r| (r.1.clone(), r.2.clone())).collect()
    }
}

fn client(dir: &std::path::Path, name: &str) -> Client {
    let mut cfg = ClientConfig::new(dir.join(format!("client-{name}")));
    cfg.allow_insecure_http = true;
    cfg.device_name = format!("e2e-{name}");
    Client::open(cfg).unwrap()
}

fn has_text(c: &Client, conv: &str, text: &str) -> bool {
    c.timeline(conv, 100, None).iter().any(|t| !t.message.deleted && t.message.body == text)
}

async fn tg(http: &reqwest::Client, url: &str, token: &str, method: &str, body: Value) -> Value {
    let r = http.post(format!("{url}/bot{token}/{method}")).json(&body).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["ok"], true, "{method}: {v}");
    v["result"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sv24_bots_webhooks_forms_schedule_ai() {
    if std::env::var("ATOMIC_E2E").is_err() {
        eprintln!("SV-24 phase 3 skipped: run tests/e2e/run.sh (needs docker)");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let _g = Guard;
    let _ = sh(&["rm", "-f", "-v", NAME]);
    let public = format!("http://127.0.0.1:{PORT}");
    let (ok, out) = sh(&[
        "run", "-d", "--name", NAME, "--add-host", "host.docker.internal:host-gateway", "-p", &format!("127.0.0.1:{PORT}:8080"),
        "-e", &format!("ATOMIC_PUBLIC_URL={public}"), "-e", "ATOMIC_LOCKOUT_THRESHOLD=50", "-e", "ATOMIC_WORKER_TICK_MS=300", IMAGE,
    ]);
    assert!(ok, "server failed to start: {out}");
    wait_health(&public).await;
    let (fake, fport) = Fake::start().await;
    let host = format!("http://host.docker.internal:{fport}");
    let http = reqwest::Client::new();

    let admin = AdminClient::setup(&public, &SetupRequest { username: "admin".into(), display_name: "Admin".into(), password: "test-admin-pw".into(), email: None, storage: None }).await.unwrap();
    let a = admin.create_user("alice", "alice", Some("password-123")).await.unwrap().user.user.id;
    let alice = client(tmp.path(), "alice");
    alice.login(&public, "alice", "password-123", None).await.unwrap();
    alice.start();
    wait_for("alice online", || alice.connection() == ConnState::Online).await;
    let chan = admin.create_channel("finance", &[a.clone()]).await.unwrap().conversation.id;
    wait_for("alice has channel", || alice.conversations().iter().any(|c| c.conversation.id == chan)).await;
    let bot = admin.create_bot("receipts_bot", "Receipt Logger", "Logs receipts", &[chan.clone()]).await.unwrap();
    let tok = bot.token.clone();

    // --- Telegram-style: getMe, sendMessage, sendDocument ---------------------------------------------------
    assert_eq!(tg(&http, &public, &tok, "getMe", json!({})).await["username"], "receipts_bot");
    tg(&http, &public, &tok, "sendMessage", json!({"chat_id": chan, "text": "Receipt logged: $12.50"})).await;
    wait_for("alice sees bot message", || has_text(&alice, &chan, "Receipt logged: $12.50")).await;
    let m = alice.timeline(&chan, 20, None).into_iter().find(|t| t.message.body.starts_with("Receipt logged")).unwrap().message;
    assert_eq!(m.sender_id, bot.user.id);
    let form = reqwest::multipart::Form::new()
        .text("chat_id", chan.clone())
        .text("caption", "the receipt")
        .part("document", reqwest::multipart::Part::bytes(b"%PDF-1.4 fake receipt".to_vec()).file_name("receipt.pdf").mime_str("application/pdf").unwrap());
    let r: Value = http.post(format!("{public}/bot{tok}/sendDocument")).multipart(form).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["ok"], true, "{r}");
    wait_for("alice sees document", || alice.timeline(&chan, 20, None).iter().any(|t| t.message.attachments.iter().any(|x| x.name == "receipt.pdf"))).await;

    // --- getUpdates: a user message reaches the bot, offset acknowledges it -----------------------------------
    alice.send_text(&chan, "please log lunch", SendOptions::default()).unwrap();
    wait_for("update queued", || has_text(&alice, &chan, "please log lunch")).await;
    let mut ups = Value::Null;
    for _ in 0..40 {
        ups = tg(&http, &public, &tok, "getUpdates", json!({})).await;
        if ups.as_array().map(|a| a.iter().any(|u| u["message"]["text"] == "please log lunch")).unwrap_or(false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let last = ups.as_array().unwrap().iter().map(|u| u["update_id"].as_i64().unwrap()).max().unwrap();
    assert!(ups.as_array().unwrap().iter().any(|u| u["message"]["text"] == "please log lunch"));
    let after = tg(&http, &public, &tok, "getUpdates", json!({"offset": last + 1})).await;
    assert!(after.as_array().unwrap().is_empty(), "acknowledged updates are gone");

    // --- setWebhook delivery to a local receiver, secret header -----------------------------------------------
    tg(&http, &public, &tok, "setWebhook", json!({"url": format!("{host}/tg-hook"), "secret_token": "s3cr3t"})).await;
    alice.send_text(&chan, "webhook me", SendOptions::default()).unwrap();
    wait_for("telegram webhook delivered", || fake.on("/tg-hook").iter().any(|r| r.1.contains("webhook me"))).await;
    let (h, body) = fake.on("/tg-hook").into_iter().find(|r| r.1.contains("webhook me")).unwrap();
    assert_eq!(h.get("x-telegram-bot-api-secret-token").unwrap(), "s3cr3t");
    assert!(serde_json::from_str::<Value>(&body).unwrap()["update_id"].as_i64().unwrap() > 0);
    tg(&http, &public, &tok, "deleteWebhook", json!({})).await;

    // --- incoming webhook ---------------------------------------------------------------------------------------
    let inc = admin.create_incoming_webhook("CI builds", &chan).await.unwrap();
    let r = http.post(format!("{public}{}", inc.path)).json(&json!({"text": "build 42 passed"})).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 201);
    wait_for("incoming webhook message", || has_text(&alice, &chan, "build 42 passed")).await;

    // --- outgoing webhook with a verified signature ---------------------------------------------------------------
    let out = admin
        .create_outgoing_webhook(&CreateOutgoingWebhook { name: "Audit".into(), url: format!("{host}/audit"), secret: Some("topsecret".into()), events: vec!["message.created".into()], conv_id: Some(chan.clone()) })
        .await
        .unwrap();
    assert_eq!(out.secret, "topsecret");
    alice.send_text(&chan, "audit me", SendOptions::default()).unwrap();
    wait_for("outgoing delivered", || fake.on("/audit").iter().any(|r| r.1.contains("audit me"))).await;
    let (h, body) = fake.on("/audit").into_iter().find(|r| r.1.contains("audit me")).unwrap();
    let ts = h.get("x-atomic-timestamp").unwrap().to_str().unwrap().to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"topsecret").unwrap();
    mac.update(format!("{ts}.{body}").as_bytes());
    assert_eq!(h.get("x-atomic-signature").unwrap().to_str().unwrap(), format!("sha256={}", hex::encode(mac.finalize().into_bytes())));
    admin.update_outgoing_webhook(&out.id, &UpdateOutgoingWebhook { enabled: Some(false), ..Default::default() }).await.unwrap();

    // --- form submit round trip --------------------------------------------------------------------------------------
    let sent = tg(
        &http, &public, &tok, "sendMessage",
        json!({"chat_id": chan, "text": "Approve?", "form": {"id": "exp", "title": "Expense", "fields": [{"id": "ref", "type": "text", "label": "Reference", "required": true}], "buttons": [{"id": "ok", "label": "Approve", "style": "primary"}]}}),
    )
    .await;
    let mid = sent["message_id"].as_str().unwrap().to_string();
    wait_for("form arrives", || alice.timeline(&chan, 30, None).iter().any(|t| t.message.id == mid && t.message.form.is_some())).await;
    let vals: BTreeMap<String, Value> = serde_json::from_value(json!({"ref": "INV-7"})).unwrap();
    alice.submit_form(&mid, None, vals).await.unwrap();
    let mut got = false;
    for _ in 0..40 {
        let ups = tg(&http, &public, &tok, "getUpdates", json!({})).await;
        if ups.as_array().unwrap().iter().any(|u| u.get("callback_query").is_some()) {
            got = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(got, "the bot received the form response");

    // --- scheduled message survives a server restart ---------------------------------------------------------------------
    let due = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 + 6_000;
    let s = alice.schedule_message(&chan, "scheduled across restart", due, None).await.unwrap();
    assert!(!has_text(&alice, &chan, "scheduled across restart"));
    let (ok, out) = sh(&["restart", "-t", "1", NAME]);
    assert!(ok, "{out}");
    wait_health(&public).await;
    wait_for("alice reconnects", || alice.connection() == ConnState::Online).await;
    wait_for("scheduled message sent after restart", || has_text(&alice, &chan, "scheduled across restart")).await;
    let _ = s;

    // --- AI search with a fake provider -----------------------------------------------------------------------------------------
    assert!(!alice.refresh_server_meta().await.unwrap().ai_search_enabled, "AI is off by default");
    assert!(alice.ai_search("invoice", false).await.is_err());
    admin
        .put_ai_settings(&AiSettingsUpdate { enabled: Some(true), api_key: Some("sk-fake-0000".into()), base_url: Some(host.clone()), ..Default::default() })
        .await
        .unwrap();
    assert!(alice.refresh_server_meta().await.unwrap().ai_search_enabled);
    alice.send_text(&chan, "invoice 99 is paid", SendOptions::default()).unwrap();
    wait_for("invoice message", || has_text(&alice, &chan, "invoice 99 is paid")).await;
    let r = alice.ai_search("find the invoice", false).await.unwrap();
    assert_eq!(r.interpreted.q, "invoice");
    assert!(r.results.hits.iter().any(|h| h.message.body == "invoice 99 is paid"));
    let calls = fake.on("/v1/messages");
    assert!(!calls.is_empty());
    assert_eq!(calls[0].0.get("x-api-key").unwrap(), "sk-fake-0000");
}

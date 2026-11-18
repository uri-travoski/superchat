//! Phase 3: bots (Telegram-style and native), webhooks, forms, App Home, scheduled messages and
//! AI features, end to end through the real server and client-core. All external services are
//! local fakes.
mod common;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::routing::any;
use axum::Router;
use common::*;
use hmac::{Hmac, Mac};
use proto::*;
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ------------------------------------------------------------------ fakes

#[derive(Clone, Debug)]
struct Req {
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Clone, Default)]
struct Receiver {
    reqs: Arc<Mutex<Vec<Req>>>,
    /// Number of initial requests to answer with HTTP 500.
    fail_first: Arc<Mutex<usize>>,
}

impl Receiver {
    async fn start(fail_first: usize) -> (Receiver, String) {
        let r = Receiver { fail_first: Arc::new(Mutex::new(fail_first)), ..Default::default() };
        async fn handle(State(r): State<Receiver>, _m: Method, u: Uri, h: HeaderMap, b: Bytes) -> (StatusCode, &'static str) {
            r.reqs.lock().unwrap().push(Req { path: u.path().to_string(), headers: h, body: String::from_utf8_lossy(&b).to_string() });
            let mut f = r.fail_first.lock().unwrap();
            if *f > 0 {
                *f -= 1;
                return (StatusCode::INTERNAL_SERVER_ERROR, "no");
            }
            (StatusCode::OK, "ok")
        }
        let app = Router::new().fallback(any(handle)).with_state(r.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (r, url)
    }
    fn all(&self) -> Vec<Req> {
        self.reqs.lock().unwrap().clone()
    }
    fn json_bodies(&self) -> Vec<Value> {
        self.all().iter().filter_map(|r| serde_json::from_str(&r.body).ok()).collect()
    }
}

/// A fake Anthropic / OpenAI endpoint. The reply depends on the system prompt.
#[derive(Clone, Default)]
struct Provider {
    reqs: Arc<Mutex<Vec<(String, HeaderMap, Value)>>>,
}

impl Provider {
    async fn start() -> (Provider, String) {
        let p = Provider::default();
        async fn handle(State(p): State<Provider>, u: Uri, h: HeaderMap, b: Bytes) -> axum::Json<Value> {
            let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
            p.reqs.lock().unwrap().push((u.path().to_string(), h, v.clone()));
            let openai = u.path().ends_with("/chat/completions");
            let system = if openai { v["messages"][0]["content"].as_str().unwrap_or("").to_string() } else { v["system"].as_str().unwrap_or("").to_string() };
            let text = if system.contains("search filter") {
                r#"Here you go: {"q": "invoice", "from": "bob", "in": "finance", "has": null, "after": "2000-01-01", "before": null}"#.to_string()
            } else if system.contains("summarise") {
                "Recap: the team talked about invoices.".to_string()
            } else if system.contains("Answer the user") {
                "Bob sent the invoice.".to_string()
            } else {
                "ok".to_string()
            };
            if openai {
                axum::Json(json!({"choices": [{"message": {"role": "assistant", "content": text}}]}))
            } else {
                axum::Json(json!({"content": [{"type": "text", "text": text}], "role": "assistant"}))
            }
        }
        let app = Router::new().fallback(any(handle)).with_state(p.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (p, url)
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

/// Telegram-style call with a JSON body.
async fn tg(s: &TestServer, token: &str, method: &str, body: Value) -> (u16, Value) {
    let r = http().post(format!("{}/bot{}/{}", s.url, token, method)).json(&body).send().await.unwrap();
    (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
}

async fn tg_ok(s: &TestServer, token: &str, method: &str, body: Value) -> Value {
    let (code, v) = tg(s, token, method, body).await;
    assert_eq!(code, 200, "{method}: {v}");
    assert_eq!(v["ok"], true, "{method}: {v}");
    v["result"].clone()
}

struct Env {
    s: TestServer,
    alice: client_core::Client,
    a_id: String,
    chan: String,
    bot: AdminBot,
}

async fn env() -> Env {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let chan = make_channel(&s, "finance", &[&a_id]).await;
    wait_conv(&alice, &chan).await;
    let bot = s.admin.create_bot("receipts_bot", "Receipt Logger", "Logs receipts", &[chan.clone()]).await.unwrap();
    // The bot joined: alice sees the member and the bot as a user flagged is_bot.
    wait_until("bot user known to alice", || alice.user(&bot.user.id).map(|u| u.is_bot).unwrap_or(false)).await;
    Env { s, alice, a_id, chan, bot }
}

fn hmac_hex(secret: &str, ts: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{ts}.{body}").as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

// ------------------------------------------------------------------ bots

#[tokio::test]
async fn telegram_send_edit_delete_and_updates() {
    let e = env().await;
    let tok = e.bot.token.clone();
    assert!(tok.starts_with("bt_"));
    let me = tg_ok(&e.s, &tok, "getMe", json!({})).await;
    assert_eq!(me["is_bot"], true);
    assert_eq!(me["username"], "receipts_bot");

    // sendMessage as JSON; alice sees it with an app (bot) sender.
    let m = tg_ok(&e.s, &tok, "sendMessage", json!({"chat_id": e.chan, "text": "Receipt logged: $12.50", "parse_mode": "Markdown"})).await;
    let mid = m["message_id"].as_str().unwrap().to_string();
    assert_eq!(m["chat"]["id"], json!(e.chan));
    let got = wait_msg(&e.alice, &e.chan, "Receipt logged: $12.50").await;
    assert_eq!(got.sender_id, e.bot.user.id);
    assert!(e.alice.user(&got.sender_id).unwrap().is_bot);

    // Form-encoded and query-string parameters work like Telegram's.
    let r = http()
        .post(format!("{}/bot{}/sendMessage", e.s.url, tok))
        .form(&[("chat_id", e.chan.as_str()), ("text", "<b>bold</b> &amp; plain"), ("parse_mode", "HTML")])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    wait_msg(&e.alice, &e.chan, "*bold* & plain").await;
    let r = http().get(format!("{}/bot{}/sendMessage?chat_id={}&text=via+query", e.s.url, tok, e.chan)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    wait_msg(&e.alice, &e.chan, "via query").await;

    // editMessageText / deleteMessage on its own message.
    tg_ok(&e.s, &tok, "editMessageText", json!({"chat_id": e.chan, "message_id": mid, "text": "Receipt logged: $13.00"})).await;
    wait_msg(&e.alice, &e.chan, "Receipt logged: $13.00").await;
    assert!(e.alice.timeline(&e.chan, 50, None).iter().any(|t| t.message.id == mid && t.message.is_edited()));
    assert_eq!(tg_ok(&e.s, &tok, "deleteMessage", json!({"chat_id": e.chan, "message_id": mid})).await, json!(true));
    wait_until("tombstone", || e.alice.timeline(&e.chan, 50, None).iter().any(|t| t.message.id == mid && t.message.deleted)).await;

    // A bot cannot edit someone else's message.
    let alice_msg = say(&e.alice, &e.chan, "alice says hi").await;
    let (code, v) = tg(&e.s, &tok, "editMessageText", json!({"message_id": alice_msg.id, "text": "hijack"})).await;
    assert_eq!((code, v["ok"].as_bool()), (403, Some(false)), "{v}");
    assert_eq!(v["error_code"], 403);

    // getUpdates: alice's message is queued; offset confirms it.
    let ups = tg_ok(&e.s, &tok, "getUpdates", json!({})).await;
    let ups = ups.as_array().unwrap();
    assert_eq!(ups.len(), 1, "{ups:?}");
    assert_eq!(ups[0]["message"]["text"], "alice says hi");
    assert_eq!(ups[0]["message"]["from"]["username"], "alice");
    let next = ups[0]["update_id"].as_i64().unwrap() + 1;
    let ups = tg_ok(&e.s, &tok, "getUpdates", json!({"offset": next})).await;
    assert!(ups.as_array().unwrap().is_empty());

    // Long poll: returns as soon as a message arrives.
    let (s_url, tok2, chan2) = (e.s.url.clone(), tok.clone(), e.chan.clone());
    let poll = tokio::spawn(async move {
        let r = http().post(format!("{s_url}/bot{tok2}/getUpdates")).json(&json!({"offset": next, "timeout": 20})).send().await.unwrap();
        let _ = chan2;
        r.json::<Value>().await.unwrap()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let started = std::time::Instant::now();
    say(&e.alice, &e.chan, "wake the poller").await;
    let v = poll.await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(v["result"][0]["message"]["text"], "wake the poller");

    // Errors have Telegram's shape; a bad token is 401; unknown chat is 400.
    let (c, v) = tg(&e.s, "bt_nope", "getMe", json!({})).await;
    assert_eq!((c, v["ok"].as_bool(), v["error_code"].as_i64()), (401, Some(false), Some(401)));
    let (c, v) = tg(&e.s, &tok, "sendMessage", json!({"chat_id": "does-not-exist", "text": "x"})).await;
    assert_eq!((c, v["ok"].as_bool()), (400, Some(false)));
    assert!(v["description"].as_str().unwrap().starts_with("Bad Request"));
    let (c, v) = tg(&e.s, &tok, "sendMessage", json!({"chat_id": e.chan})).await;
    assert_eq!(c, 400, "{v}");
    let (c, _) = tg(&e.s, &tok, "noSuchMethod", json!({})).await;
    assert_eq!(c, 400);
}

#[tokio::test]
async fn bots_post_only_where_they_are_members_and_tokens_can_be_revoked() {
    let e = env().await;
    let other = make_channel(&e.s, "private", &[&e.a_id]).await;
    wait_conv(&e.alice, &other).await;
    let tok = e.bot.token.clone();
    let (c, v) = tg(&e.s, &tok, "sendMessage", json!({"chat_id": other, "text": "intruder"})).await;
    assert_eq!(c, 400, "{v}");
    assert!(texts(&e.alice, &other).is_empty());
    // Native API: same rule.
    let r = http().post(format!("{}{}/bot/messages", e.s.url, API_PREFIX)).bearer_auth(&tok).json(&json!({"conv_id": other, "body": "intruder"})).send().await.unwrap();
    assert_eq!(r.status(), 404);
    // Added by the admin: now it works; removed: it stops again.
    e.s.admin.add_bot_to(&e.bot.user.id, &other).await.unwrap();
    tg_ok(&e.s, &tok, "sendMessage", json!({"chat_id": other, "text": "welcome"})).await;
    wait_msg(&e.alice, &other, "welcome").await;
    e.s.admin.remove_bot_from(&e.bot.user.id, &other).await.unwrap();
    let (c, _) = tg(&e.s, &tok, "sendMessage", json!({"chat_id": other, "text": "again"})).await;
    assert_eq!(c, 400);

    // A new token invalidates the old one.
    let fresh = e.s.admin.regenerate_bot_token(&e.bot.user.id).await.unwrap();
    assert_ne!(fresh.token, tok);
    assert_eq!(tg(&e.s, &tok, "getMe", json!({})).await.0, 401);
    tg_ok(&e.s, &fresh.token, "getMe", json!({})).await;
    // Disabling the bot stops it, enabling restores it.
    e.s.admin.update_bot(&e.bot.user.id, &UpdateBotRequest { disabled: Some(true), ..Default::default() }).await.unwrap();
    assert_eq!(tg(&e.s, &fresh.token, "getMe", json!({})).await.0, 401);
    e.s.admin.update_bot(&e.bot.user.id, &UpdateBotRequest { disabled: Some(false), ..Default::default() }).await.unwrap();
    tg_ok(&e.s, &fresh.token, "getMe", json!({})).await;
    // A bot has no password login.
    let r = http().post(format!("{}{}/login", e.s.url, API_PREFIX)).json(&json!({"username": "receipts_bot", "password": "!bot"})).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn telegram_documents_file_ids_and_downloads() {
    let e = env().await;
    let tok = e.bot.token.clone();
    let part = reqwest::multipart::Part::bytes(b"receipt body bytes".to_vec()).file_name("receipt.txt").mime_str("text/plain").unwrap();
    let form = reqwest::multipart::Form::new().text("chat_id", e.chan.clone()).text("caption", "March receipt").part("document", part);
    let r = http().post(format!("{}/bot{}/sendDocument", e.s.url, tok)).multipart(form).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["document"]["file_name"], "receipt.txt");
    let msg = wait_msg(&e.alice, &e.chan, "March receipt").await;
    assert_eq!(msg.attachments[0].name, "receipt.txt");
    let path = e.alice.download_attachment(&msg.attachments[0]).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"receipt body bytes");

    // Re-send by file_id (as a photo alias with an image type is not needed: documents too).
    let fid = v["result"]["document"]["file_id"].as_str().unwrap().to_string();
    let again = tg_ok(&e.s, &tok, "sendDocument", json!({"chat_id": e.chan, "document": fid, "caption": "again"})).await;
    assert_ne!(again["document"]["file_id"], json!(fid), "the copy is its own attachment");
    wait_msg(&e.alice, &e.chan, "again").await;

    // getFile + download through /file/bot<token>/...
    let gf = tg_ok(&e.s, &tok, "getFile", json!({"file_id": fid})).await;
    assert_eq!(gf["file_size"], 18);
    let bytes = http().get(format!("{}/file/bot{}/{}", e.s.url, tok, gf["file_path"].as_str().unwrap())).send().await.unwrap();
    assert_eq!(bytes.status(), 200);
    assert_eq!(bytes.bytes().await.unwrap().as_ref(), b"receipt body bytes");
    assert_eq!(http().get(format!("{}/file/botbt_bad/{}", e.s.url, gf["file_path"].as_str().unwrap())).send().await.unwrap().status(), 401);

    // Aliases: sendPhoto / sendVoice take the same shape and map to photo / voice in updates.
    let png = reqwest::multipart::Part::bytes(vec![0x89, b'P', b'N', b'G', 1, 2, 3]).file_name("p.png").mime_str("image/png").unwrap();
    let r = http().post(format!("{}/bot{}/sendPhoto", e.s.url, tok)).multipart(reqwest::multipart::Form::new().text("chat_id", e.chan.clone()).part("photo", png)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.json::<Value>().await.unwrap()["result"]["photo"][0]["file_id"].is_string());
    let ogg = reqwest::multipart::Part::bytes(vec![1, 2, 3, 4]).file_name("v.ogg").mime_str("audio/ogg").unwrap();
    let r = http().post(format!("{}/bot{}/sendVoice", e.s.url, tok)).multipart(reqwest::multipart::Form::new().text("chat_id", e.chan.clone()).part("voice", ogg)).send().await.unwrap();
    assert!(r.json::<Value>().await.unwrap()["result"]["voice"]["mime_type"].as_str().unwrap().starts_with("audio/ogg"));

    // A user file sent to the bot shows up in its updates and can be fetched by file_id.
    let f = e.s.dir.path().join("invoice.pdf");
    std::fs::write(&f, b"%PDF-fake").unwrap();
    e.alice.send_files(&e.chan, "please log", &[f], client_core::SendOptions::default()).unwrap();
    wait_msg(&e.alice, &e.chan, "please log").await;
    let ups = tg_ok(&e.s, &tok, "getUpdates", json!({})).await;
    let up = ups.as_array().unwrap().iter().find(|u| u["message"]["caption"] == "please log").expect("update with file");
    let fid = up["message"]["document"]["file_id"].as_str().unwrap();
    let gf = tg_ok(&e.s, &tok, "getFile", json!({"file_id": fid})).await;
    assert_eq!(gf["file_size"], 9);
    // A bot cannot read files of conversations it is not in.
    let other = make_channel(&e.s, "elsewhere", &[&e.a_id]).await;
    wait_conv(&e.alice, &other).await;
    let f2 = e.s.dir.path().join("hidden.txt");
    std::fs::write(&f2, b"hidden").unwrap();
    e.alice.send_files(&other, "secret file", &[f2], client_core::SendOptions::default()).unwrap();
    let hidden = wait_msg(&e.alice, &other, "secret file").await;
    let (c, _) = tg(&e.s, &tok, "getFile", json!({"file_id": hidden.attachments[0].id})).await;
    assert_eq!(c, 400);
}

#[tokio::test]
async fn telegram_webhook_delivery_with_secret_and_retries() {
    let e = env().await;
    let tok = e.bot.token.clone();
    let (rx, url) = Receiver::start(2).await; // first two deliveries fail with 500
    let (c, v) = tg(&e.s, &tok, "setWebhook", json!({"url": "ftp://nope"})).await;
    assert_eq!(c, 400, "{v}");
    tg_ok(&e.s, &tok, "setWebhook", json!({"url": format!("{url}/hook"), "secret_token": "s3cr3t"})).await;
    let info = tg_ok(&e.s, &tok, "getWebhookInfo", json!({})).await;
    assert_eq!(info["url"], json!(format!("{url}/hook")));

    // getUpdates is refused while a webhook is set, like Telegram.
    let (c, _) = tg(&e.s, &tok, "getUpdates", json!({})).await;
    assert_eq!(c, 409);

    say(&e.alice, &e.chan, "hello bot").await;
    wait_until("webhook delivered after retries", || rx.all().len() >= 3).await;
    let reqs = rx.all();
    assert_eq!(reqs[0].path, "/hook");
    for r in &reqs {
        assert_eq!(r.headers.get("x-telegram-bot-api-secret-token").unwrap(), "s3cr3t");
    }
    let body: Value = serde_json::from_str(&reqs[2].body).unwrap();
    assert_eq!(body["message"]["text"], "hello bot");
    assert!(body["update_id"].as_i64().unwrap() > 0);
    // Delivered once: no more requests after success.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(rx.all().len(), 3);
    let info = tg_ok(&e.s, &tok, "getWebhookInfo", json!({})).await;
    assert_eq!(info["pending_update_count"], 0);

    // deleteWebhook returns to polling.
    tg_ok(&e.s, &tok, "deleteWebhook", json!({})).await;
    assert_eq!(tg_ok(&e.s, &tok, "getWebhookInfo", json!({})).await["url"], "");
    say(&e.alice, &e.chan, "polling again").await;
    let ups = tg_ok(&e.s, &tok, "getUpdates", json!({})).await;
    assert_eq!(ups[0]["message"]["text"], "polling again");
}

#[tokio::test]
async fn native_bot_api_mirrors_the_telegram_one() {
    let e = env().await;
    let auth = |rb: reqwest::RequestBuilder| rb.bearer_auth(&e.bot.token);
    let base = format!("{}{}/bot", e.s.url, API_PREFIX);
    let me: Value = auth(http().get(format!("{base}/me"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(me["user"]["username"], "receipts_bot");
    let r = auth(http().post(format!("{base}/messages"))).json(&json!({"conv_id": e.chan, "body": "native hello", "client_id": "k1"})).send().await.unwrap();
    assert_eq!(r.status(), 201);
    let m: Value = r.json().await.unwrap();
    wait_msg(&e.alice, &e.chan, "native hello").await;
    // Idempotent on client_id.
    let r = auth(http().post(format!("{base}/messages"))).json(&json!({"conv_id": e.chan, "body": "native hello", "client_id": "k1"})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let m2: Value = r.json().await.unwrap();
    assert_eq!(m["id"], m2["id"]);
    let r = auth(http().patch(format!("{base}/messages/{}", m["id"].as_str().unwrap()))).json(&json!({"body": "native edited"})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    wait_msg(&e.alice, &e.chan, "native edited").await;
    let convs: Value = auth(http().get(format!("{base}/conversations"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(convs.as_array().unwrap().len(), 1);
    let hist: Value = auth(http().get(format!("{base}/conversations/{}/messages", e.chan))).send().await.unwrap().json().await.unwrap();
    assert_eq!(hist["messages"][0]["body"], "native edited");
    say(&e.alice, &e.chan, "ping native").await;
    let ups: Value = auth(http().get(format!("{base}/updates?timeout=5"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(ups[0]["message"]["text"], "ping native");
    let r = auth(http().delete(format!("{base}/messages/{}", m["id"].as_str().unwrap()))).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(http().get(format!("{base}/me")).send().await.unwrap().status(), 401);
    // A normal user token is not a bot token.
    let tok = e.alice.api().unwrap().current_access_token().unwrap();
    assert_eq!(http().get(format!("{base}/me")).bearer_auth(tok).send().await.unwrap().status(), 401);
}

// ------------------------------------------------------------------ webhooks

#[tokio::test]
async fn incoming_webhook_posts_as_an_app() {
    let e = env().await;
    let hook = e.s.admin.create_incoming_webhook("CI", &e.chan).await.unwrap();
    assert!(hook.path.starts_with("/api/v1/hooks/"));
    let url = format!("{}{}", e.s.url, hook.path);
    let r = http().post(&url).json(&json!({"text": "build *passed*"})).send().await.unwrap();
    assert_eq!(r.status(), 201);
    let m = wait_msg(&e.alice, &e.chan, "build *passed*").await;
    assert_eq!(m.sender_id, hook.bot_user_id);
    let sender = e.alice.user(&m.sender_id).unwrap();
    assert!(sender.is_bot);
    assert_eq!(sender.display_name, "CI");
    // Wrong secret, then disabled.
    let bad = format!("{}{}/hooks/{}/wrong", e.s.url, API_PREFIX, hook.id);
    assert_eq!(http().post(&bad).json(&json!({"text": "x"})).send().await.unwrap().status(), 401);
    let r = e.s.admin.call::<IncomingWebhook>(reqwest::Method::PATCH, &format!("/admin/webhooks/incoming/{}", hook.id), Some(json!({"enabled": false}))).await.unwrap();
    assert!(!r.enabled);
    assert_eq!(http().post(&url).json(&json!({"text": "x"})).send().await.unwrap().status(), 403);
    // Regenerating the secret kills the old URL.
    let r = e.s.admin.call::<IncomingWebhook>(reqwest::Method::PATCH, &format!("/admin/webhooks/incoming/{}", hook.id), Some(json!({"enabled": true, "regenerate_secret": true}))).await.unwrap();
    assert_ne!(r.path, hook.path);
    assert_eq!(http().post(&url).json(&json!({"text": "x"})).send().await.unwrap().status(), 401);
    assert_eq!(http().post(format!("{}{}", e.s.url, r.path)).json(&json!({"text": "new url"})).send().await.unwrap().status(), 201);
    // Listed for the admin.
    let all = e.s.admin.webhooks().await.unwrap();
    assert_eq!(all.incoming.len(), 1);
    assert!(all.incoming[0].last_used_ms.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn outgoing_webhook_is_signed_filtered_and_retried() {
    let e = env().await;
    let other = make_channel(&e.s, "other", &[&e.a_id]).await;
    wait_conv(&e.alice, &other).await;
    let (rx, url) = Receiver::start(1).await; // first delivery fails
    let hook = e
        .s
        .admin
        .create_outgoing_webhook(&CreateOutgoingWebhook { name: "audit".into(), url: format!("{url}/in"), secret: Some("topsecret".into()), events: vec!["message.created".into()], conv_id: Some(e.chan.clone()) })
        .await
        .unwrap();
    assert_eq!(hook.secret, "topsecret");
    assert!(e.s.admin.create_outgoing_webhook(&CreateOutgoingWebhook { name: "x".into(), url: "ftp://bad".into(), secret: None, events: vec![], conv_id: None }).await.is_err());
    assert!(e.s.admin.create_outgoing_webhook(&CreateOutgoingWebhook { name: "x".into(), url: url.clone(), secret: None, events: vec!["no.such".into()], conv_id: None }).await.is_err());

    say(&e.alice, &other, "not in the filter").await;
    let sent = say(&e.alice, &e.chan, "invoice 42 paid").await;
    wait_until("delivery after one retry", || rx.all().len() >= 2).await;
    let reqs = rx.all();
    assert_eq!(reqs.len(), 2, "one failure then success; other conversation filtered out");
    let last = reqs.last().unwrap();
    let ts = last.headers.get("x-atomic-timestamp").unwrap().to_str().unwrap();
    assert_eq!(last.headers.get("x-atomic-signature").unwrap().to_str().unwrap(), hmac_hex("topsecret", ts, &last.body), "HMAC over timestamp.body");
    assert_ne!(last.headers.get("x-atomic-signature").unwrap().to_str().unwrap(), hmac_hex("wrong", ts, &last.body));
    assert_eq!(last.headers.get("x-atomic-event").unwrap(), "message.created");
    let body: Value = serde_json::from_str(&last.body).unwrap();
    assert_eq!(body["event"], "message.created");
    assert_eq!(body["message"]["id"], json!(sent.id));
    assert_eq!(body["message"]["body"], "invoice 42 paid");
    assert_eq!(body["conversation"]["name"], "finance");
    assert_eq!(body["sender"]["username"], "alice");

    // Edits are not in this hook's event list; widening it picks up updates and deletes.
    e.alice.edit_message(&sent.id, "invoice 42 paid (edited)").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rx.all().len(), 2);
    e.s.admin.update_outgoing_webhook(&hook.id, &UpdateOutgoingWebhook { events: Some(vec![]), ..Default::default() }).await.unwrap();
    e.alice.edit_message(&sent.id, "invoice 42 paid (edited twice)").await.unwrap();
    wait_until("update event", || rx.json_bodies().iter().any(|b| b["event"] == "message.updated")).await;
    e.alice.delete_message(&sent.id).await.unwrap();
    wait_until("delete event", || rx.json_bodies().iter().any(|b| b["event"] == "message.deleted")).await;

    // Test ping, status and disabling.
    e.s.admin.test_outgoing_webhook(&hook.id).await.unwrap();
    wait_until("ping", || rx.json_bodies().iter().any(|b| b["event"] == "ping")).await;
    wait_until("status shows delivered", || {
        let w = futures_executor_block(e.s.admin.webhooks()).unwrap();
        w.outgoing[0].last_status.as_deref().map(|s| s.starts_with("delivered")).unwrap_or(false)
    })
    .await;
    e.s.admin.update_outgoing_webhook(&hook.id, &UpdateOutgoingWebhook { enabled: Some(false), ..Default::default() }).await.unwrap();
    let n = rx.all().len();
    say(&e.alice, &e.chan, "after disable").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rx.all().len(), n);
}

fn futures_executor_block<F: std::future::Future>(f: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

#[tokio::test(flavor = "multi_thread")]
async fn webhook_gives_up_after_max_attempts_and_reports_failure() {
    let e = env().await;
    let (rx, url) = Receiver::start(1000).await; // always failing
    let hook = e.s.admin.create_outgoing_webhook(&CreateOutgoingWebhook { name: "dead".into(), url: format!("{url}/x"), secret: None, events: vec![], conv_id: None }).await.unwrap();
    say(&e.alice, &e.chan, "will never arrive").await;
    // webhook_max_attempts is 4 in tests.
    wait_until("four attempts", || rx.all().iter().filter(|r| r.body.contains("will never arrive")).count() >= 4).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(rx.all().iter().filter(|r| r.body.contains("will never arrive")).count(), 4);
    let w = e.s.admin.webhooks().await.unwrap();
    let w = w.outgoing.iter().find(|w| w.id == hook.id).unwrap();
    assert_eq!(w.failed, 1);
    assert!(w.last_status.as_deref().unwrap().starts_with("failed"));
}

// ------------------------------------------------------------------ forms

fn expense_form() -> Value {
    json!({
        "id": "expense",
        "title": "Approve expense",
        "description": "Receipt from Bunnings",
        "fields": [
            {"type": "text", "id": "ref", "label": "Reference", "required": true},
            {"type": "multiline", "id": "note", "label": "Note"},
            {"type": "boolean", "id": "urgent", "label": "Urgent"},
            {"type": "select", "id": "cat", "label": "Category", "options": [{"value": "tools", "label": "Tools"}, {"value": "food", "label": "Food"}]},
            {"type": "multi_select", "id": "tags", "label": "Tags", "options": [{"value": "a"}, {"value": "b"}, {"value": "c"}]}
        ],
        "buttons": [
            {"id": "approve", "label": "Approve", "style": "primary"},
            {"id": "reject", "label": "Reject", "style": "danger", "form": {"id": "why", "title": "Why?", "fields": [{"type": "text", "id": "reason", "label": "Reason", "required": true}]}}
        ],
        "submit_label": "Send"
    })
}

#[tokio::test]
async fn forms_round_trip_to_the_bot() {
    let e = env().await;
    let tok = e.bot.token.clone();
    let sent = tg_ok(&e.s, &tok, "sendMessage", json!({"chat_id": e.chan, "text": "Please review", "form": expense_form()})).await;
    let mid = sent["message_id"].as_str().unwrap().to_string();
    let m = wait_msg(&e.alice, &e.chan, "Please review").await;
    let form = m.form.clone().expect("form arrives with the message");
    assert_eq!(form.fields.len(), 5);
    assert_eq!(form.buttons[1].form.as_ref().unwrap().fields[0].id, "reason");

    let vals = |v: Value| -> BTreeMap<String, Value> { serde_json::from_value(v).unwrap() };
    // Validation: required, unknown select value, unknown field.
    let err = e.alice.submit_form(&mid, None, vals(json!({"note": "x"}))).await.unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::Validation), "{err}");
    assert!(err.to_string().contains("Reference"));
    assert!(e.alice.submit_form(&mid, None, vals(json!({"ref": "r1", "cat": "cars"}))).await.is_err());
    assert!(e.alice.submit_form(&mid, None, vals(json!({"ref": "r1", "zzz": "1"}))).await.is_err());
    assert!(e.alice.submit_form(&mid, None, vals(json!({"ref": "r1", "tags": ["a", "zz"]}))).await.is_err());
    // Modal button needs its own required field.
    assert!(e.alice.submit_form(&mid, Some("reject"), vals(json!({}))).await.is_err());
    assert!(e.alice.submit_form(&mid, Some("nope"), vals(json!({}))).await.is_err());

    // Good submission.
    let done = e.alice.submit_form(&mid, None, vals(json!({"ref": "INV-7", "note": "ok", "urgent": true, "cat": "tools", "tags": ["a", "c"]}))).await.unwrap();
    assert_eq!(done.responded, vec![e.a_id.clone()]);
    assert!(e.alice.i_responded(&done));
    // Only one response per person.
    let err = e.alice.submit_form(&mid, Some("approve"), vals(json!({}))).await.unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::Conflict));

    // The bot received a callback_query with the values.
    let ups = tg_ok(&e.s, &tok, "getUpdates", json!({})).await;
    let cq = ups.as_array().unwrap().iter().find(|u| u.get("callback_query").is_some()).expect("callback update");
    let cq = &cq["callback_query"];
    assert_eq!(cq["from"]["username"], "alice");
    assert_eq!(cq["data"], "expense");
    assert_eq!(cq["values"]["ref"], "INV-7");
    assert_eq!(cq["values"]["tags"], json!(["a", "c"]));
    assert_eq!(cq["values"]["urgent"], true);
    assert_eq!(cq["message"]["message_id"], json!(mid));

    // The bot can replace the form (clears responses) and use reply_markup keyboards.
    tg_ok(&e.s, &tok, "editMessageText", json!({"message_id": mid, "text": "Approved", "form": null})).await;
    let kb = tg_ok(
        &e.s,
        &tok,
        "sendMessage",
        json!({"chat_id": e.chan, "text": "Deploy?", "reply_markup": {"inline_keyboard": [[{"text": "Yes", "callback_data": "yes"}, {"text": "No", "callback_data": "no"}]]}}),
    )
    .await;
    let kb_id = kb["message_id"].as_str().unwrap().to_string();
    let m = wait_msg(&e.alice, &e.chan, "Deploy?").await;
    assert_eq!(m.form.unwrap().buttons.len(), 2);
    e.alice.submit_form(&kb_id, Some("yes"), BTreeMap::new()).await.unwrap();
    let ups = tg_ok(&e.s, &tok, "getUpdates", json!({})).await;
    assert!(ups.as_array().unwrap().iter().any(|u| u["callback_query"]["data"] == "yes"));

    // Modal button with its own form.
    let m2 = tg_ok(&e.s, &tok, "sendMessage", json!({"chat_id": e.chan, "text": "Review 2", "form": expense_form()})).await;
    let id2 = m2["message_id"].as_str().unwrap().to_string();
    e.alice.submit_form(&id2, Some("reject"), vals(json!({"reason": "too dear"}))).await.unwrap();
    let ups = tg_ok(&e.s, &tok, "getUpdates", json!({})).await;
    let cq = ups.as_array().unwrap().iter().find(|u| u["callback_query"]["data"] == "reject").unwrap();
    assert_eq!(cq["callback_query"]["values"]["reason"], "too dear");

    // Malformed forms are refused when sent.
    let (c, _) = tg(&e.s, &tok, "sendMessage", json!({"chat_id": e.chan, "text": "bad", "form": {"id": "f", "fields": [{"type": "select", "id": "s"}]}})).await;
    assert_eq!(c, 400);
}

#[tokio::test]
async fn app_home_blocks_and_submissions() {
    let e = env().await;
    let tok = e.bot.token.clone();
    let home = json!({"blocks": [
        {"type": "header", "text": "Receipt Logger"},
        {"type": "text", "text": "Logs *receipts* from the team."},
        {"type": "divider"},
        {"type": "button", "id": "report", "label": "Monthly report", "style": "primary"},
        {"type": "form", "form": {"id": "add", "title": "Add receipt", "fields": [{"type": "text", "id": "amount", "label": "Amount", "required": true}], "submit_label": "Log it"}}
    ]});
    tg_ok(&e.s, &tok, "setAppHome", json!({"home": home})).await;
    let p = e.alice.bot_profile(&e.bot.user.id).await.unwrap();
    assert_eq!(p.description, "Logs receipts");
    assert_eq!(p.home.blocks.len(), 5);
    assert!(matches!(&p.home.blocks[0], HomeBlock::Header { text } if text == "Receipt Logger"));
    assert_eq!(e.alice.bot_profiles().await.unwrap().len(), 1);

    e.alice.home_submit(&e.bot.user.id, None, Some("report"), BTreeMap::new()).await.unwrap();
    let vals: BTreeMap<String, Value> = serde_json::from_value(json!({"amount": "9.99"})).unwrap();
    e.alice.home_submit(&e.bot.user.id, Some("add"), None, vals).await.unwrap();
    assert!(e.alice.home_submit(&e.bot.user.id, None, Some("nope"), BTreeMap::new()).await.is_err());
    assert!(e.alice.home_submit(&e.bot.user.id, Some("add"), None, BTreeMap::new()).await.is_err(), "amount required");
    let ups = tg_ok(&e.s, &tok, "getUpdates", json!({})).await;
    let ups = ups.as_array().unwrap();
    assert_eq!(ups.len(), 2);
    assert_eq!(ups[0]["callback_query"]["home"], true);
    assert_eq!(ups[0]["callback_query"]["data"], "report");
    assert_eq!(ups[1]["callback_query"]["values"]["amount"], "9.99");
    // Invalid home is refused.
    let (c, _) = tg(&e.s, &tok, "setAppHome", json!({"home": {"blocks": [{"type": "nonsense"}]}})).await;
    assert_eq!(c, 400);
}

// ------------------------------------------------------------------ scheduled

fn in_ms(ms: i64) -> i64 {
    proto_now() + ms
}

fn proto_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[tokio::test]
async fn scheduled_messages_send_at_due_time_and_can_be_cancelled() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "team", &[&a_id, &b_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&bob, &ch).await;

    assert!(alice.schedule_message(&ch, "in the past", proto_now() - 1000, None).await.is_err());
    assert!(alice.schedule_message(&ch, "  ", in_ms(5000), None).await.is_err());
    let keep = alice.schedule_message(&ch, "scheduled hello", in_ms(1500), None).await.unwrap();
    let drop = alice.schedule_message(&ch, "scheduled then cancelled", in_ms(1500), None).await.unwrap();
    assert_eq!(alice.scheduled_pending(None).len(), 2);
    assert_eq!(alice.scheduled_pending(Some(&ch)).len(), 2);
    assert!(bob.refresh_scheduled().await.unwrap().is_empty(), "scheduled messages are private to their author");
    let c = alice.cancel_scheduled(&drop.id).await.unwrap();
    assert_eq!(c.state, ScheduledState::Cancelled);
    assert!(alice.cancel_scheduled(&drop.id).await.is_err(), "already cancelled");
    assert!(bob.cancel_scheduled(&keep.id).await.is_err(), "not bob's");

    // Nothing yet; then it arrives as alice's message, once.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!texts(&bob, &ch).contains(&"scheduled hello".to_string()));
    let m = wait_msg(&bob, &ch, "scheduled hello").await;
    assert_eq!(m.sender_id, a_id);
    wait_until("state sent", || alice.scheduled().iter().any(|x| x.id == keep.id && x.state == ScheduledState::Sent)).await;
    let sent = alice.scheduled().into_iter().find(|x| x.id == keep.id).unwrap();
    assert_eq!(sent.message_id.as_deref(), Some(m.id.as_str()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(texts(&bob, &ch).iter().filter(|t| *t == "scheduled hello").count(), 1);
    assert!(!texts(&bob, &ch).contains(&"scheduled then cancelled".to_string()));
    // Server-side listing agrees and a second device sees the same list.
    let alice2 = s.new_client("alice-phone");
    alice2.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    wait_until("second device lists the history", || alice2.scheduled().len() == 2).await;
}

#[tokio::test]
async fn scheduled_messages_survive_a_server_restart() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "team", &[&a_id, &b_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&bob, &ch).await;
    let due_soon = alice.schedule_message(&ch, "survives restart", in_ms(2500), None).await.unwrap();
    let overdue = alice.schedule_message(&ch, "due while down", in_ms(1200), None).await.unwrap();
    assert_ne!(due_soon.id, overdue.id);

    // Stop the server (workers included) before anything is due, then start a new one on the same data.
    s.st.shutdown();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut cfg = atomic_server::config::Config::for_test(s.dir.path().join("server"));
    cfg.worker_tick_ms = 100;
    // Wait past the first due time: nothing may have been sent while no server was running.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let st2 = atomic_server::build_state(cfg).await.unwrap();
    let url2 = atomic_server::serve_for_test(st2.clone()).await.unwrap();
    assert!(st2.db.with(|c| Ok(atomic_server::scheduled::list(c, &a_id)?)).unwrap().iter().all(|m| m.state == ScheduledState::Pending || m.state == ScheduledState::Sent));
    let bob2 = {
        let mut cc = client_core::ClientConfig::new(s.dir.path().join("client-bob2"));
        cc.allow_insecure_http = true;
        let c = client_core::Client::open(cc).unwrap();
        c.login(&url2, "bob", "password-123", None).await.unwrap();
        c.start();
        wait_online(&c).await;
        c
    };
    wait_msg(&bob2, &ch, "due while down").await;
    wait_msg(&bob2, &ch, "survives restart").await;
    let n = texts(&bob2, &ch).iter().filter(|t| t.as_str() == "survives restart" || t.as_str() == "due while down").count();
    assert_eq!(n, 2, "each exactly once");
}

// ------------------------------------------------------------------ AI

#[tokio::test]
async fn ai_is_off_by_default_and_works_with_a_fake_provider() {
    let (prov, url) = Provider::start().await;
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let fin = make_channel(&s, "finance", &[&a_id, &b_id]).await;
    wait_conv(&alice, &fin).await;
    wait_conv(&bob, &fin).await;
    say(&bob, &fin, "invoice 42 is paid").await;
    say(&alice, &fin, "thanks, lunch at noon").await;

    // Off by default: meta says so and the endpoints refuse.
    assert!(!alice.refresh_server_meta().await.unwrap().ai_search_enabled);
    assert!(!alice.ai_enabled());
    assert_eq!(alice.ai_search("what did bob send", false).await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    assert_eq!(alice.ai_recap(&fin, None).await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    let cfg = s.admin.ai_overview().await.unwrap();
    assert!(cfg.providers.is_empty() && cfg.search_provider.is_none());
    assert!(cfg.notice.contains("sent to the provider"));
    // Cannot enable without a key; invalid kind refused; members cannot configure.
    let input = |name: &str, kind: &str, key: &str, base: &str| AiProviderInput { name: name.into(), kind: kind.into(), api_key: key.into(), base_url: Some(base.into()), ..Default::default() };
    assert!(s.admin.add_ai_provider(&input("Claude", "anthropic", "", &url)).await.is_err());
    assert!(s.admin.add_ai_provider(&input("Claude", "acme", "sk-x", &url)).await.is_err());
    let r = http().get(format!("{}{}/admin/ai", s.url, API_PREFIX)).bearer_auth(alice.api().unwrap().current_access_token().unwrap()).send().await.unwrap();
    assert_eq!(r.status(), 403);

    // Anthropic provider through the fake server; the first provider becomes the default for both features.
    let claude = s.admin.add_ai_provider(&input("Claude", "anthropic", "sk-ant-fake-1234567", &url)).await.unwrap();
    assert!(claude.enabled && claude.has_key);
    assert_eq!(claude.model, "claude-sonnet-5-5");
    assert_eq!(claude.key_hint, "4567");
    let ov = s.admin.ai_overview().await.unwrap();
    assert_eq!(ov.search_provider.as_deref(), Some(claude.id.as_str()));
    assert_eq!(ov.recap_provider.as_deref(), Some(claude.id.as_str()));
    assert!(!format!("{ov:?}").contains("sk-ant-fake"), "the key is never returned");
    assert!(alice.refresh_server_meta().await.unwrap().ai_search_enabled);
    let test = s.admin.test_ai_provider(&claude.id).await.unwrap();
    assert!(test.ok, "{test:?}");
    assert!(s.admin.test_ai_provider("nope").await.is_err());

    let r = alice.ai_search("what did bob send about invoices last week", true).await.unwrap();
    assert_eq!(r.interpreted.q, "invoice");
    assert_eq!(r.interpreted.from.as_deref(), Some("bob"));
    assert_eq!(r.interpreted.in_conv.as_deref(), Some(fin.as_str()), "channel name mapped to its id");
    assert!(r.interpreted.after_ms.is_some());
    assert_eq!(r.results.hits.len(), 1);
    assert_eq!(r.results.hits[0].message.body, "invoice 42 is paid");
    assert_eq!(r.answer.as_deref(), Some("Bob sent the invoice."));
    let rc = alice.ai_recap(&fin, None).await.unwrap();
    assert_eq!(rc.message_count, 2);
    assert!(rc.summary.starts_with("Recap:"));
    assert_eq!(alice.ai_recap("nope", None).await.unwrap_err().code(), Some(ErrorCode::NotFound));

    {
        let reqs = prov.reqs.lock().unwrap();
        let (path, headers, body) = reqs.iter().find(|r| r.2["system"].as_str().unwrap_or("").contains("search filter")).unwrap();
        assert_eq!(path, "/v1/messages");
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-ant-fake-1234567");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
        assert_eq!(body["model"], "claude-sonnet-5-5");
        // Search interpretation sends the question and names, never message bodies.
        assert!(!body.to_string().contains("invoice 42 is paid"));
        // The recap does send message text.
        let recap = reqs.iter().find(|r| r.2["system"].as_str().unwrap_or("").contains("summarise")).unwrap();
        assert!(recap.2.to_string().contains("invoice 42 is paid"));
    }

    // OpenAI-compatible provider: switching kind and model of the same entry (the stored key is kept).
    let upd = s.admin.update_ai_provider(&claude.id, &AiProviderUpdate { kind: Some("openai-compatible".into()), model: Some("local-model".into()), ..Default::default() }).await.unwrap();
    assert_eq!(upd.kind, "openai");
    assert_eq!(upd.key_hint, "4567");
    let rc = alice.ai_recap(&fin, None).await.unwrap();
    assert!(rc.summary.starts_with("Recap:"));
    {
        let reqs = prov.reqs.lock().unwrap();
        let last = reqs.last().unwrap();
        assert_eq!(last.0, "/v1/chat/completions");
        assert_eq!(last.1.get("authorization").unwrap(), "Bearer sk-ant-fake-1234567");
        assert_eq!(last.2["model"], "local-model");
    }
    // Recap respects membership and a time window.
    let (_c_id, carol) = s.user("carol").await;
    assert_eq!(carol.ai_recap(&fin, None).await.unwrap_err().code(), Some(ErrorCode::NotFound));
    let none = alice.ai_recap(&fin, Some(proto_now() + 60_000)).await.unwrap();
    assert_eq!(none.message_count, 0);
    // Disabling the only provider turns the features off; deleting it removes it and its default pointers.
    s.admin.update_ai_provider(&claude.id, &AiProviderUpdate { enabled: Some(false), ..Default::default() }).await.unwrap();
    assert!(!alice.refresh_server_meta().await.unwrap().ai_search_enabled);
    assert!(alice.ai_search("x", false).await.is_err());
    let ov = s.admin.delete_ai_provider(&claude.id).await.unwrap();
    assert!(ov.providers.is_empty() && ov.search_provider.is_none());
}

#[tokio::test]
async fn ai_providers_route_per_feature_fail_over_and_migrate() {
    let (prov_a, url_a) = Provider::start().await;
    let (prov_b, url_b) = Provider::start().await;
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let ch = make_channel(&s, "finance", &[&a_id]).await;
    wait_conv(&alice, &ch).await;
    say(&alice, &ch, "invoice 7 sent").await;
    let input = |name: &str, kind: &str, key: &str, base: &str| AiProviderInput { name: name.into(), kind: kind.into(), api_key: key.into(), base_url: Some(base.into()), ..Default::default() };
    let a = s.admin.add_ai_provider(&input("Provider A", "anthropic", "key-aaaaaaaaaaaa", &url_a)).await.unwrap();
    let b = s.admin.add_ai_provider(&input("Provider B", "openai-compatible", "key-bbbbbbbbbbbb", &url_b)).await.unwrap();
    assert!(s.admin.add_ai_provider(&input("provider a", "anthropic", "k", &url_a)).await.is_err(), "names are unique");
    assert_eq!(s.admin.ai_overview().await.unwrap().providers.len(), 2);
    // Default: A for both. Then recap goes to B, search stays on A.
    s.admin.put_ai_defaults(&AiDefaultsUpdate { recap_provider: Some(b.id.clone()), ..Default::default() }).await.unwrap();
    assert!(s.admin.put_ai_defaults(&AiDefaultsUpdate { search_provider: Some("nope".into()), ..Default::default() }).await.is_err());
    alice.ai_recap(&ch, None).await.unwrap();
    assert_eq!(prov_a.reqs.lock().unwrap().len(), 0);
    assert_eq!(prov_b.reqs.lock().unwrap().len(), 1);
    assert_eq!(prov_b.reqs.lock().unwrap()[0].0, "/v1/chat/completions");
    alice.ai_search("invoice", false).await.unwrap();
    assert_eq!(prov_a.reqs.lock().unwrap().len(), 1);
    assert_eq!(prov_a.reqs.lock().unwrap()[0].0, "/v1/messages");
    assert_eq!(prov_b.reqs.lock().unwrap().len(), 1);
    // Each provider can be tested on its own, even when disabled.
    assert!(s.admin.test_ai_provider(&b.id).await.unwrap().ok);
    // Failover: the default recap provider goes dark, the other one answers.
    s.admin.update_ai_provider(&b.id, &AiProviderUpdate { base_url: Some("http://127.0.0.1:9".into()), ..Default::default() }).await.unwrap();
    assert!(!s.admin.test_ai_provider(&b.id).await.unwrap().ok);
    let before = prov_a.reqs.lock().unwrap().len();
    let rc = alice.ai_recap(&ch, None).await.unwrap();
    assert!(rc.summary.starts_with("Recap:"));
    assert_eq!(prov_a.reqs.lock().unwrap().len(), before + 1, "fell back to provider A");
    // Disabled providers are skipped; with none left the features refuse.
    s.admin.update_ai_provider(&b.id, &AiProviderUpdate { enabled: Some(false), ..Default::default() }).await.unwrap();
    s.admin.update_ai_provider(&a.id, &AiProviderUpdate { enabled: Some(false), ..Default::default() }).await.unwrap();
    assert_eq!(alice.ai_recap(&ch, None).await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    // Provider list survives deletion of the default.
    let ov = s.admin.delete_ai_provider(&a.id).await.unwrap();
    assert_eq!(ov.providers.len(), 1);
    assert!(ov.search_provider.is_none());
}

#[tokio::test]
async fn single_provider_settings_of_1_0_migrate_into_the_list() {
    let (prov, url) = Provider::start().await;
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let ch = make_channel(&s, "finance", &[&a_id]).await;
    wait_conv(&alice, &ch).await;
    say(&alice, &ch, "invoice 8 sent").await;
    // The 1.0 record, written the way 1.0 stored it.
    let old = json!({"enabled": true, "provider": "openai", "model": "legacy-model", "base_url": url, "api_key": "sk-legacy-123456789"}).to_string();
    s.st.db.with(|c| atomic_server::db::set_setting(c, "ai_settings", &old)).unwrap();
    let ov = s.admin.ai_overview().await.unwrap();
    assert_eq!(ov.providers.len(), 1);
    let p = &ov.providers[0];
    assert_eq!((p.kind.as_str(), p.model.as_str(), p.enabled, p.has_key), ("openai", "legacy-model", true, true));
    assert_eq!(ov.search_provider.as_deref(), Some(p.id.as_str()));
    assert!(alice.refresh_server_meta().await.unwrap().ai_search_enabled);
    alice.ai_recap(&ch, None).await.unwrap();
    assert_eq!(prov.reqs.lock().unwrap()[0].1.get("authorization").unwrap(), "Bearer sk-legacy-123456789");
    // Migrated once: the old record no longer holds the key.
    let left = s.st.db.with(|c| atomic_server::db::get_setting(c, "ai_settings")).unwrap().unwrap();
    assert!(!left.contains("sk-legacy"));
    assert_eq!(s.admin.ai_overview().await.unwrap().providers.len(), 1);
}

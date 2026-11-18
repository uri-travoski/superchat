//! SV-24: end-to-end run against the real server in docker, MinIO (S3) and a fake Google Drive.
//!
//! Run with `tests/e2e/run.sh` (builds the image, sets ATOMIC_E2E=1). Without ATOMIC_E2E the test
//! is skipped so `cargo test --workspace` needs no docker.
//!
//! Covers: login, DM, channel, edit rule, delete tombstone, upload/download, sync after offline,
//! disabled-account wipe, search isolation, storage move + verify (S3 -> Google Drive -> S3),
//! and clear-history with automatic backup. Server data is stored in MinIO / fake Drive throughout.
//!
//! Phase 2 additions (run in the same scenario, before the disabled-account step): search filters,
//! threads and pins, a fake ntfy server receiving a content-free wake-up, an encrypted full backup
//! to MinIO, restore onto a second, empty server container (old chats and attachments come back),
//! selective restore, date-range delete and bulk delete with a backup first.
mod fake_gdrive;

use client_core::admin::AdminClient;
use client_core::{Client, ClientConfig, ConnState, SendOptions, SendState};
use proto::*;
use std::process::Command;
use std::time::Duration;

const SERVER_PORT: u16 = 18300;
const MINIO_PORT: u16 = 18310;
const SERVER2_PORT: u16 = 18301;
const IMAGE: &str = "superchat-server:e2e";
const MINIO_IMAGE: &str = "bitnamilegacy/minio:latest";

fn sh(cmd: &str, args: &[&str]) -> (bool, String) {
    let o = Command::new(cmd).args(args).output().expect("run command");
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

/// Removes the containers when the test ends, pass or fail.
struct Containers(Vec<String>);
impl Drop for Containers {
    fn drop(&mut self) {
        for n in &self.0 {
            let _ = sh("docker", &["rm", "-f", "-v", n]);
        }
        let _ = sh("docker", &["network", "rm", "superchat-e2e"]);
    }
}

async fn wait_for<F: FnMut() -> bool>(what: &str, mut f: F) {
    for _ in 0..400 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for: {what}");
}

async fn wait_http(url: &str) {
    for _ in 0..200 {
        if let Ok(r) = reqwest_get(url).await {
            if r {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{url} never became healthy");
}

/// Plain-socket GET so the e2e crate needs no HTTP client of its own. True for any HTTP response.
async fn reqwest_get(url: &str) -> std::io::Result<bool> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let hostport = url.trim_start_matches("http://").split('/').next().unwrap().to_string();
    let path = format!("/{}", url.trim_start_matches("http://").split_once('/').map(|x| x.1).unwrap_or(""));
    let mut s = tokio::net::TcpStream::connect(&hostport).await?;
    s.write_all(format!("GET {path} HTTP/1.0\r\nHost: {hostport}\r\n\r\n").as_bytes()).await?;
    let mut buf = String::new();
    s.read_to_string(&mut buf).await?;
    Ok(buf.starts_with("HTTP/1."))
}

fn client(dir: &std::path::Path, name: &str) -> Client {
    let mut cfg = ClientConfig::new(dir.join(format!("client-{name}")));
    cfg.allow_insecure_http = true;
    cfg.device_name = format!("e2e-{name}");
    Client::open(cfg).unwrap()
}

async fn user(a: &AdminClient, url: &str, dir: &std::path::Path, name: &str) -> (String, Client) {
    let r = a.create_user(name, name, Some("password-123")).await.unwrap();
    let c = client(dir, name);
    c.login(url, name, "password-123", None).await.unwrap();
    c.start();
    wait_for("online", || c.connection() == ConnState::Online).await;
    (r.user.user.id, c)
}

fn texts(c: &Client, conv: &str) -> Vec<String> {
    c.timeline(conv, 200, None).into_iter().filter(|t| !t.message.deleted).map(|t| t.message.body).collect()
}

async fn sent(c: &Client, conv: &str, text: &str) {
    c.send_text(conv, text, SendOptions::default()).unwrap();
    wait_for("send accepted", || c.timeline(conv, 50, None).iter().any(|t| t.message.body == text && t.state != SendState::Sending && !t.local)).await;
}

fn minio_files() -> usize {
    let (_, out) = sh("docker", &["exec", "superchat-e2e-minio", "sh", "-c", "find /data/superchat -name xl.meta | wc -l"]);
    out.trim().parse().unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sv24_full_scenario() {
    if std::env::var("ATOMIC_E2E").is_err() {
        eprintln!("SV-24 skipped: run tests/e2e/run.sh (needs docker)");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let _guard = Containers(vec!["superchat-e2e-minio".into(), "superchat-e2e-server".into(), "superchat-e2e-server2".into()]);
    let _ = sh("docker", &["rm", "-f", "-v", "superchat-e2e-minio", "superchat-e2e-server", "superchat-e2e-server2"]);
    let _ = sh("docker", &["network", "rm", "superchat-e2e"]);
    let (ok, out) = sh("docker", &["network", "create", "superchat-e2e"]);
    assert!(ok, "network: {out}");

    // --- infrastructure -------------------------------------------------------------------------
    let (ok, out) = sh(
        "docker",
        &[
            "run", "-d", "--name", "superchat-e2e-minio", "--network", "superchat-e2e", "--user", "0", "-p", &format!("127.0.0.1:{MINIO_PORT}:9000"),
            "-e", "MINIO_ROOT_USER=e2eaccess", "-e", "MINIO_ROOT_PASSWORD=e2esecret-1234",
            "--entrypoint", "sh", MINIO_IMAGE, "-c", "mkdir -p /data/superchat && exec /opt/bitnami/minio/bin/minio server /data",
        ],
    );
    assert!(ok, "minio failed to start: {out}");
    let (gdrive, gport) = fake_gdrive::start().await;
    let public = format!("http://127.0.0.1:{SERVER_PORT}");
    let (ok, out) = sh(
        "docker",
        &[
            "run", "-d", "--name", "superchat-e2e-server", "--network", "superchat-e2e", "--add-host", "host.docker.internal:host-gateway",
            "-p", &format!("127.0.0.1:{SERVER_PORT}:8080"), "-e", &format!("ATOMIC_PUBLIC_URL={public}"),
            "-e", "ATOMIC_LOCKOUT_THRESHOLD=50", IMAGE,
        ],
    );
    assert!(ok, "server failed to start (build the image first): {out}");
    wait_http(&format!("{public}/api/v1/health")).await;
    wait_http(&format!("http://127.0.0.1:{MINIO_PORT}/minio/health/live")).await;

    let s3 = StorageConfig::S3 {
        provider: S3Provider::Minio,
        endpoint: Some("http://superchat-e2e-minio:9000".to_string()),
        region: "us-east-1".into(),
        bucket: "superchat".into(),
        access_key_id: "e2eaccess".into(),
        secret_access_key: "e2esecret-1234".into(),
        prefix: None,
        path_style: true,
        allow_http: true,
    };
    let gd = StorageConfig::Gdrive {
        client_id: "cid".into(),
        client_secret: "csecret".into(),
        refresh_token: "fake-refresh-token".into(),
        folder_name: Some("SuperChat e2e".into()),
        api_base: Some(format!("http://host.docker.internal:{gport}")),
        token_url: Some(format!("http://host.docker.internal:{gport}/token")),
    };

    // --- first-run setup: owner + S3 storage chosen in the wizard ------------------------------------
    let admin = AdminClient::setup(
        &public,
        &SetupRequest { username: "admin".into(), display_name: "Admin".into(), password: "test-admin-pw".into(), email: None, storage: Some(s3.clone()) },
    )
    .await
    .expect("setup with MinIO storage");
    let t = admin.test_storage(&s3).await.unwrap();
    assert!(t.ok, "{}", t.message);
    assert_eq!(admin.storage().await.unwrap().config.kind(), "s3");
    // A second setup is refused.
    assert!(AdminClient::setup(&public, &SetupRequest { username: "x".into(), display_name: "x".into(), password: "password-123".into(), email: None, storage: None }).await.is_err());

    // --- login / users ---------------------------------------------------------------------------------
    let (a_id, alice) = user(&admin, &public, tmp.path(), "alice").await;
    let (b_id, bob) = user(&admin, &public, tmp.path(), "bob").await;
    let (c_id, carol) = user(&admin, &public, tmp.path(), "carol").await;
    // Wrong password is rejected with a stable code.
    let bad = client(tmp.path(), "bad");
    assert_eq!(bad.login(&public, "alice", "nope-nope", None).await.unwrap_err().code(), Some(ErrorCode::InvalidCredentials));

    // --- DM + channel + edit rule + tombstone --------------------------------------------------------------
    let ab = alice.open_dm(&b_id).await.unwrap();
    sent(&alice, &ab, "hello bob").await;
    wait_for("bob got dm", || texts(&bob, &ab) == vec!["hello bob"]).await;
    let first = alice.timeline(&ab, 5, None)[0].message.clone();
    assert!(alice.edit_message(&first.id, "hello bob!").await.is_ok());
    wait_for("bob sees edit", || texts(&bob, &ab) == vec!["hello bob!"]).await;
    sent(&bob, &ab, "hi alice").await;
    wait_for("alice got reply", || texts(&alice, &ab).len() == 2).await;
    assert_eq!(alice.edit_message(&first.id, "too late").await.unwrap_err().code(), Some(ErrorCode::EditWindowClosed), "DM edit closes after the other side replies");

    let ch = admin.create_channel("project", &[a_id.clone(), b_id.clone()]).await.unwrap().conversation.id;
    wait_for("members have channel", || alice.conversations().iter().any(|c| c.conversation.id == ch) && bob.conversations().iter().any(|c| c.conversation.id == ch)).await;
    assert!(!carol.conversations().iter().any(|c| c.conversation.id == ch), "channels are private");
    sent(&alice, &ch, "kickoff at 10").await;
    sent(&bob, &ch, "ack").await;
    let kick = alice.timeline(&ch, 10, None)[0].message.clone();
    assert!(alice.edit_message(&kick.id, "kickoff at 11").await.is_ok(), "channel messages stay editable");
    wait_for("bob sees channel edit", || texts(&bob, &ch).contains(&"kickoff at 11".to_string())).await;
    let bobs = bob.timeline(&ch, 10, None).into_iter().find(|t| t.message.body == "ack").unwrap().message;
    bob.delete_message(&bobs.id).await.unwrap();
    wait_for("alice sees tombstone", || alice.timeline(&ch, 10, None).iter().any(|t| t.message.deleted && t.message.id == bobs.id)).await;

    // --- upload / download through MinIO ---------------------------------------------------------------------
    let data: Vec<u8> = (0..700_000u32).map(|i| (i % 253) as u8).collect();
    let f = tmp.path().join("report.pdf");
    std::fs::write(&f, &data).unwrap();
    alice.send_files(&ab, "the report", std::slice::from_ref(&f), SendOptions::default()).unwrap();
    wait_for("bob has file", || bob.timeline(&ab, 10, None).iter().any(|t| !t.message.attachments.is_empty())).await;
    let att = bob.timeline(&ab, 10, None).into_iter().find(|t| !t.message.attachments.is_empty()).unwrap().message.attachments[0].clone();
    assert_eq!(att.size, data.len() as i64);
    assert_eq!(std::fs::read(bob.download_attachment(&att).await.unwrap()).unwrap(), data);
    assert_eq!(carol.download_attachment(&att).await.unwrap_err().code(), Some(ErrorCode::NotFound), "non-members cannot fetch files");
    assert!(minio_files() >= 1, "the file is physically in MinIO");
    // A second, private file only alice sees (notes to self) for the clear-history step later.
    let me_conv = alice.self_conversation().unwrap().conversation.id;
    let nf = tmp.path().join("private.bin");
    std::fs::write(&nf, vec![9u8; 4096]).unwrap();
    alice.send_files(&me_conv, "", &[nf], SendOptions::default()).unwrap();
    wait_for("private file stored", || alice.timeline(&me_conv, 10, None).iter().any(|t| !t.local && !t.message.attachments.is_empty())).await;

    // --- sync after offline -------------------------------------------------------------------------------------
    bob.stop();
    for i in 1..=5 {
        alice.send_text(&ch, &format!("offline-{i}"), SendOptions::default()).unwrap();
    }
    wait_for("alice delivered all", || alice.timeline(&ch, 50, None).iter().filter(|t| t.message.body.starts_with("offline-") && !t.local).count() == 5).await;
    assert!(!texts(&bob, &ch).iter().any(|t| t.starts_with("offline-")));
    bob.start();
    wait_for("bob caught up by cursor", || texts(&bob, &ch).iter().filter(|t| t.starts_with("offline-")).count() == 5).await;

    // --- search isolation ---------------------------------------------------------------------------------------------
    sent(&alice, &ab, "the zebra plan").await;
    wait_for("bob has zebra", || texts(&bob, &ab).iter().any(|t| t.contains("zebra"))).await;
    assert_eq!(alice.search("zebra").await.unwrap().hits.len(), 1);
    assert_eq!(bob.search("zebra").await.unwrap().hits.len(), 1);
    assert_eq!(carol.search("zebra").await.unwrap().hits.len(), 0, "carol shares no chat with alice and bob");

    // --- storage move + verify: S3 -> Google Drive (fake) -> S3 ----------------------------------------------------------------
    let before = admin.storage().await.unwrap();
    assert!(before.attachments >= 2);
    let t = admin.test_storage(&gd).await.unwrap();
    assert!(t.ok, "gdrive test: {}", t.message);
    let job = admin.migrate_storage(&gd, true).await.unwrap();
    let job = admin.wait_job(&job.id).await.unwrap();
    assert_eq!(job.state, "done", "{job:?}");
    assert_eq!(admin.storage().await.unwrap().config.kind(), "gdrive");
    assert!(gdrive.count_files() as i64 >= before.attachments, "files landed in the fake Drive: {:?}", gdrive.names());
    assert_eq!(minio_files(), 0, "source copies were removed after verification");
    let v = admin.verify_storage(true).await.unwrap();
    assert_eq!(admin.wait_job(&v.id).await.unwrap().state, "done");
    // Downloads still work, now served from Drive (also a Range read through the signed path).
    assert_eq!(std::fs::read(bob.download_attachment(&att).await.unwrap()).unwrap(), data);
    sent(&bob, &ab, "after the move").await;
    let job = admin.migrate_storage(&s3, true).await.unwrap();
    let job = admin.wait_job(&job.id).await.unwrap();
    assert_eq!(job.state, "done", "{job:?}");
    assert_eq!(gdrive.count_files(), 0, "Drive emptied after moving back");
    assert!(minio_files() >= 2);
    assert_eq!(std::fs::read(bob.download_attachment(&att).await.unwrap()).unwrap(), data);

    // --- CLI smoke (scripted use of the same client-core) ----------------------------------------------------------------------------
    let cli = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/atomic");
    if cli.exists() {
        let run = |args: &[&str]| {
            let o = Command::new(&cli).args(["--insecure-http", "--data-dir"]).arg(tmp.path().join("cli")).args(args).output().unwrap();
            assert!(o.status.success(), "atomic {args:?}: {}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).to_string()
        };
        run(&["login", &public, "carol", "password-123"]);
        let who = run(&["whoami"]);
        assert!(who.contains("carol"), "{who}");
        let dm = run(&["dm", "alice"]).trim().to_string();
        run(&["send", &dm, "sent from the CLI"]);
        wait_for("alice got CLI message", || alice.conversations().iter().any(|c| c.conversation.id == dm) && texts(&alice, &dm).contains(&"sent from the CLI".to_string())).await;
        assert!(run(&["history", &dm]).contains("sent from the CLI"));
        run(&["logout"]);
    } else {
        eprintln!("CLI smoke skipped: build it with `cargo build -p atomic-cli`");
    }

    phase2(&admin, tmp.path(), &s3, (&a_id, &alice), (&b_id, &bob), &carol, &ch, &ab, &att, &data).await;

    // --- disabled account: kicked immediately and wiped -----------------------------------------------------------------------------------
    let mut rx = carol.subscribe();
    admin.set_disabled(&c_id, true).await.unwrap();
    let mut got = None;
    for _ in 0..100 {
        if let Ok(Ok(client_core::ClientEvent::SignedOut { code, .. })) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            got = code;
            break;
        }
    }
    assert_eq!(got, Some(ErrorCode::AccountDisabled));
    assert!(!carol.signed_in());
    assert!(carol.conversations().is_empty() && carol.users().is_empty(), "local cache wiped");
    admin.set_disabled(&c_id, false).await.unwrap();

    // --- clear history: automatic backup first, own copy hidden, others untouched, files moved ---------------------------------------------------
    let n_backups = admin.backups().await.unwrap().len();
    let r = admin.clear_history(&a_id).await.unwrap();
    assert!(r.messages_hidden > 0);
    assert!(r.attachments_moved >= 1, "the private file moved into the backup folder");
    let backups = admin.backups().await.unwrap();
    assert_eq!(backups.len(), n_backups + 1, "a backup was written first");
    assert!(backups.iter().any(|b| b.key == r.backup_key));
    wait_for("alice signed out", || !alice.signed_in()).await;
    assert!(alice.conversations().is_empty());
    // Bob keeps his copy of the DM and the channel.
    assert!(texts(&bob, &ab).contains(&"hello bob!".to_string()));
    assert!(texts(&bob, &ch).contains(&"kickoff at 11".to_string()));
    // Alice signs in again: her history is empty, but the DM shows only new messages.
    alice.login(&public, "alice", "password-123", None).await.unwrap();
    alice.start();
    wait_for("alice online", || alice.connection() == ConnState::Online).await;
    wait_for("alice sees nothing old", || alice.conversations().iter().all(|c| texts(&alice, &c.conversation.id).is_empty())).await;
    assert_eq!(alice.search("zebra").await.unwrap().hits.len(), 0);
    sent(&bob, &ab, "fresh after clear").await;
    wait_for("alice sees only new", || texts(&alice, &ab) == vec!["fresh after clear"]).await;
    let _ = b_id;
}

/// A minimal fake ntfy server: records every request line and body, answers 200.
async fn fake_ntfy() -> (std::sync::Arc<std::sync::Mutex<Vec<String>>>, u16) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let log: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let l = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let log2 = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = l.accept().await else { return };
            let log = log2.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut got = 0;
                // Read until the headers and the 4-byte "wake" body have arrived (or the peer pauses).
                while let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(300), sock.read(&mut buf[got..])).await {
                    if n == 0 {
                        break;
                    }
                    got += n;
                    if String::from_utf8_lossy(&buf[..got]).contains("\r\n\r\nwake") {
                        break;
                    }
                }
                log.lock().unwrap().push(String::from_utf8_lossy(&buf[..got]).to_string());
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}").await;
            });
        }
    });
    (log, port)
}

#[allow(clippy::too_many_arguments)]
async fn phase2(
    admin: &AdminClient,
    tmp: &std::path::Path,
    s3: &StorageConfig,
    (a_id, alice): (&String, &Client),
    (b_id, bob): (&String, &Client),
    carol: &Client,
    ch: &str,
    ab: &str,
    att: &Attachment,
    data: &[u8],
) {
    use client_core::logic::searchq;

    // --- search filters + isolation ------------------------------------------------------------------
    sent(alice, ch, "budget review http://example.com/q4").await;
    sent(bob, ch, "budget numbers").await;
    wait_for("bob has budget", || texts(bob, ch).iter().any(|t| t.starts_with("budget review"))).await;
    let q = |s: &str| searchq::parse(s, 0);
    assert_eq!(alice.search_filtered(&q("budget")).await.unwrap().hits.len(), 2);
    assert_eq!(alice.search_filtered(&q("budget from:bob")).await.unwrap().hits.len(), 1);
    assert_eq!(alice.search_filtered(&q("budget has:link in:#project")).await.unwrap().hits.len(), 1);
    assert_eq!(alice.search_filtered(&q("budget before:01/01/2000")).await.unwrap().hits.len(), 0);
    assert_eq!(carol.search_filtered(&q("budget")).await.unwrap().hits.len(), 0, "carol is not in #project");
    assert!(!alice.search_filtered(&q("report")).await.unwrap().files.is_empty(), "file names are searchable");
    assert!(carol.search_filtered(&q("report")).await.unwrap().files.is_empty());

    // --- threads and pins -----------------------------------------------------------------------------------
    let root = alice.timeline(ch, 50, None).into_iter().find(|t| t.message.body.starts_with("budget review")).unwrap().message;
    bob.send_thread_reply(ch, &root.id, "reply in thread").unwrap();
    wait_for("alice thread", || alice.threads().iter().any(|t| t.root.id == root.id && t.unread >= 1)).await;
    assert!(alice.timeline(ch, 50, None).iter().all(|t| t.message.body != "reply in thread"));
    alice.follow_thread(&root.id, false).await.unwrap();
    assert!(alice.threads().iter().all(|t| t.root.id != root.id), "unfollow removes it from the Threads list");

    // --- push: fake ntfy receives one content-free wake-up when bob is offline ------------------------------------
    let (ntfy_log, ntfy_port) = fake_ntfy().await;
    admin.put_push_settings(&PushSettings { ntfy_url: Some(format!("http://host.docker.internal:{ntfy_port}")), ..Default::default() }).await.unwrap();
    bob.register_push(&PushRegistration { id: String::new(), kind: "ntfy".into(), target: "bob-e2e-topic".into(), ntfy_url: None, device_name: "e2e".into(), created_ms: 0 }).await.unwrap();
    bob.stop();
    tokio::time::sleep(Duration::from_millis(800)).await;
    sent(alice, ab, "top secret wake test").await;
    wait_for("ntfy wake", || !ntfy_log.lock().unwrap().is_empty()).await;
    {
        let log = ntfy_log.lock().unwrap();
        assert!(log[0].starts_with("POST /bob-e2e-topic"), "{}", log[0]);
        assert!(log.iter().all(|r| !r.contains("secret") && !r.to_lowercase().contains("alice")), "push must be content free: {log:?}");
    }
    bob.start();
    wait_for("bob back online", || bob.connection() == ConnState::Online).await;
    wait_for("bob caught up", || texts(bob, ab).iter().any(|t| t.contains("top secret wake test"))).await;

    // --- encrypted full backup to MinIO, then restore onto a second EMPTY server --------------------------------------------
    let backup_target = match s3 {
        StorageConfig::S3 { .. } => {
            let mut t = s3.clone();
            if let StorageConfig::S3 { prefix, .. } = &mut t {
                *prefix = Some("e2e-backups".into());
            }
            t
        }
        _ => unreachable!(),
    };
    admin
        .put_backup_settings(&BackupSettingsUpdate { enabled: Some(true), interval_hours: Some(24), retention: Some(3), target: Some(backup_target.clone()), passphrase: Some("e2e backup passphrase".into()), ..Default::default() })
        .await
        .unwrap();
    assert!(admin.test_backup_target().await.unwrap().ok);
    let full = admin.backup_now().await.unwrap();
    assert!(full.encrypted && full.size > 0);
    // Objects in MinIO are encrypted: the plain-text message must not appear in any backup object.
    let (_, grep) = sh("docker", &["exec", "superchat-e2e-minio", "sh", "-c", "grep -rl 'top secret wake test' /data/superchat/e2e-backups 2>/dev/null | wc -l"]);
    assert_eq!(grep.trim(), "0", "backup objects contain no plain text");
    let (ok, out) = sh(
        "docker",
        &[
            "run", "-d", "--name", "superchat-e2e-server2", "--network", "superchat-e2e", "-p", &format!("127.0.0.1:{SERVER2_PORT}:8080"),
            "-e", &format!("ATOMIC_PUBLIC_URL=http://127.0.0.1:{SERVER2_PORT}"), "-e", "ATOMIC_LOCKOUT_THRESHOLD=50", IMAGE,
        ],
    );
    assert!(ok, "second server failed to start: {out}");
    let public2 = format!("http://127.0.0.1:{SERVER2_PORT}");
    wait_http(&format!("{public2}/api/v1/health")).await;
    let r = AdminClient::fresh_restore(&public2, &FreshRestoreRequest { target: Some(backup_target.clone()), key: None, passphrase: "e2e backup passphrase".into() }).await.expect("fresh restore");
    assert!(r.users >= 4 && r.messages >= 10, "{r:?}");
    assert!(r.attachments >= 2 && r.attachments_resolvable == r.attachments, "restored server reconnects to the existing attachments: {r:?}");
    let admin2 = AdminClient::login(&public2, "admin", "test-admin-pw").await.expect("owner login on the restored server");
    let alice2 = client(tmp, "alice-restored");
    alice2.login(&public2, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_for("alice2 online", || alice2.connection() == ConnState::Online).await;
    wait_for("old chat restored", || texts(&alice2, ab).contains(&"hello bob!".to_string())).await;
    let old = alice2.timeline(ab, 50, None).into_iter().find(|t| !t.message.attachments.is_empty()).unwrap().message;
    assert_eq!(std::fs::read(alice2.download_attachment(&old.attachments[0]).await.unwrap()).unwrap(), data, "old attachment downloads through the new server");
    let _ = att;
    assert!(texts(&alice2, ch).contains(&"kickoff at 11".to_string()));

    // --- on the restored server: bulk delete (backup first), restore; date-range delete; selective restore ------------------------
    let bob2 = client(tmp, "bob-restored");
    bob2.login(&public2, "bob", "password-123", None).await.unwrap();
    bob2.start();
    wait_for("bob2 online", || bob2.connection() == ConnState::Online).await;
    wait_for("bob2 history", || texts(&bob2, ab).contains(&"hello bob!".to_string())).await;
    let target = alice2.timeline(ab, 50, None).into_iter().find(|t| t.message.body == "hi alice").unwrap().message;
    let del = admin2
        .delete_messages(&AdminDeleteMessagesRequest { message_ids: vec![target.id.clone()], conv_id: None, from_ms: None, to_ms: None, before_ms: None, scope: "everyone".into(), user_id: None })
        .await
        .unwrap();
    wait_for("tombstone", || alice2.timeline(ab, 50, None).iter().any(|t| t.message.id == target.id && t.message.deleted)).await;
    admin2.restore(&RestoreRequest { backup_id: del.backup_id, user_ids: vec![], all: false, mode: None }).await.unwrap();
    wait_for("restored message", || texts(&alice2, ab).contains(&"hi alice".to_string())).await;
    // Date-range delete (before a date in the future = everything), default keeps the other copy.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let dr = admin2
        .delete_range(b_id, &DateRangeDeleteRequest { from_ms: 0, to_ms: 0, before_ms: Some(now + 60_000), conversation_id: Some(ab.to_string()), also_other_copies: false })
        .await
        .unwrap();
    assert!(dr.messages_hidden >= 3, "{dr:?}");
    wait_for("alice2 keeps hers", || texts(&alice2, ab).len() >= 3).await;
    // Selective restore of just bob from the user backup written before the delete.
    let ub = admin2.backups().await.unwrap().into_iter().filter(|b| b.kind == "user" && b.user_id.as_deref() == Some(b_id.as_str())).max_by_key(|b| b.created_ms).unwrap();
    let rr = admin2.restore(&RestoreRequest { backup_id: ub.id, user_ids: vec![b_id.clone()], all: false, mode: Some("merge".into()) }).await.unwrap();
    assert!(rr.messages_unhidden >= 3, "{rr:?}");
    // Bob's old session was wiped by the range delete's backup? No: ranges only hide messages.
    let bob3 = client(tmp, "bob-restored-2");
    bob3.login(&public2, "bob", "password-123", None).await.unwrap();
    bob3.start();
    wait_for("bob history back", || texts(&bob3, ab).contains(&"hello bob!".to_string())).await;
    let _ = (a_id, bob2);
}

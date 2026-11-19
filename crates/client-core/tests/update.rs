//! Update checks against a fake release server (GitHub's API shape): up to date, update available, platform asset choice,
//! checksum verification (good, tampered, missing), cancel, rate limit, offline, skip and remind choices, startup/24 h schedule.

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use client_core::update::{self, Phase, Platform, Updater};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Fake {
    /// Version of the release answered (tag `v<ver>`); empty = nothing published (404).
    version: String,
    prerelease: bool,
    /// Answer 403 with rate-limit headers.
    limit: bool,
    /// The bytes the APK asset serves (the checksum asset always describes `good`).
    served: Vec<u8>,
    good: Vec<u8>,
    no_sha: bool,
    /// Bytes per second written to the downloader (0 = all at once); lets a test cancel mid-way.
    slow: bool,
    user_agents: Vec<String>,
    hits: u32,
}

type Shared = Arc<Mutex<Fake>>;

async fn latest(State(s): State<Shared>, h: HeaderMap, uri: axum::extract::OriginalUri) -> impl IntoResponse {
    let mut f = s.lock().unwrap();
    f.hits += 1;
    f.user_agents.push(h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string());
    if f.limit {
        let reset = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 600).to_string();
        return (StatusCode::FORBIDDEN, [("x-ratelimit-remaining", "0".to_string()), ("x-ratelimit-reset", reset)], "{\"message\":\"API rate limit exceeded\"}".to_string()).into_response();
    }
    if f.version.is_empty() {
        return (StatusCode::NOT_FOUND, "{\"message\":\"Not Found\"}").into_response();
    }
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let _ = uri;
    let v = f.version.clone();
    let base = format!("http://{host}/dl");
    let mut assets = vec![
        serde_json::json!({"name": format!("superchat-{v}-android.apk"), "size": f.good.len(), "browser_download_url": format!("{base}/superchat-{v}-android.apk")}),
        serde_json::json!({"name": format!("SuperChat-{v}-x86_64.AppImage"), "size": 3, "browser_download_url": format!("{base}/SuperChat-{v}-x86_64.AppImage")}),
        serde_json::json!({"name": format!("superchat_{v}-1_amd64.deb"), "size": 3, "browser_download_url": format!("{base}/superchat_{v}-1_amd64.deb")}),
    ];
    if !f.no_sha {
        assets.push(serde_json::json!({"name": format!("superchat-{v}-android.apk.sha256"), "size": 90, "browser_download_url": format!("{base}/superchat-{v}-android.apk.sha256")}));
    }
    let body = serde_json::json!({
        "tag_name": format!("v{v}"), "name": format!("SuperChat {v}"), "draft": false, "prerelease": f.prerelease,
        "html_url": format!("http://{host}/release"), "published_at": "2026-11-20T00:00:00Z",
        "body": "## What's new\n* Update checks\n* Fixes", "assets": assets
    });
    ([(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

async fn sha(State(s): State<Shared>) -> impl IntoResponse {
    let f = s.lock().unwrap();
    format!("{}  superchat-android.apk\n", hex::encode(Sha256::digest(&f.good)))
}

async fn apk(State(s): State<Shared>) -> impl IntoResponse {
    let (bytes, slow) = {
        let f = s.lock().unwrap();
        (f.served.clone(), f.slow)
    };
    if slow {
        let stream = futures_util::stream::unfold((bytes, 0usize), |(b, i)| async move {
            if i >= b.len() {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
            let end = (i + 4096).min(b.len());
            Some((Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(&b[i..end])), (b, end)))
        });
        return axum::body::Body::from_stream(stream).into_response();
    }
    bytes.into_response()
}

async fn start(f: Fake) -> (String, Shared) {
    let shared: Shared = Arc::new(Mutex::new(f));
    let app = Router::new()
        .route("/releases/latest", get(latest))
        .route("/dl/{name}", get(|State(s): State<Shared>, axum::extract::Path(n): axum::extract::Path<String>| async move {
            if n.ends_with(".sha256") { sha(State(s)).await.into_response() } else { apk(State(s)).await.into_response() }
        }))
        .with_state(shared.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}/releases/latest"), shared)
}

fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

async fn wait_for(u: &Updater, want: impl Fn(Phase) -> bool) -> update::Status {
    for _ in 0..400 {
        let s = u.status();
        if want(s.phase) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out in phase {:?}: {}", u.status().phase, u.status().error);
}

fn updater(dir: &std::path::Path, current: &str, url: &str) -> Updater {
    Updater::new(dir, tokio::runtime::Handle::current(), current, Platform::Android, url.to_string())
}

#[tokio::test]
async fn up_to_date_and_update_available() {
    let good = payload(50_000);
    let (url, fake) = start(Fake { version: "1.3.1".into(), good: good.clone(), served: good, ..Default::default() }).await;
    let d = tempfile::tempdir().unwrap();

    let u = updater(d.path(), "1.3.1", &url);
    u.check();
    let s = wait_for(&u, |p| p == Phase::UpToDate).await;
    assert!(!s.has_update() && !s.show_banner(0));
    assert!(s.last_check_ms > 0);
    // The request identifies the app by version only.
    assert_eq!(fake.lock().unwrap().user_agents, vec!["SuperChat/1.3.1".to_string()]);

    let u = updater(d.path(), "1.3.0", &url);
    u.check();
    let s = wait_for(&u, |p| p == Phase::Available).await;
    assert!(s.has_update() && s.show_banner(0));
    let r = s.release.unwrap();
    assert_eq!(r.version, "1.3.1");
    assert!(r.notes.starts_with("What's new"));
    let pick = s.pick.unwrap();
    assert_eq!(pick.asset.name, "superchat-1.3.1-android.apk");
    assert!(pick.sha256.is_some());

    // A fresh Updater (app restart) shows the cached answer at once, before any network call.
    let hits = fake.lock().unwrap().hits;
    let u2 = updater(d.path(), "1.3.0", "http://127.0.0.1:1/never");
    assert_eq!(u2.status().phase, Phase::Available);
    assert_eq!(u2.status().release.unwrap().version, "1.3.1");
    assert_eq!(fake.lock().unwrap().hits, hits);
    // ... and a cached release that this build already has is dropped.
    let u3 = updater(d.path(), "1.3.1", "http://127.0.0.1:1/never");
    assert_eq!(u3.status().phase, Phase::Idle);
}

#[tokio::test]
async fn prerelease_and_missing_release() {
    let (url, fake) = start(Fake { version: "9.0.0".into(), prerelease: true, ..Default::default() }).await;
    let d = tempfile::tempdir().unwrap();
    let u = updater(d.path(), "1.3.0", &url);
    u.check();
    assert_eq!(wait_for(&u, |p| p == Phase::UpToDate).await.release, None);
    fake.lock().unwrap().version.clear();
    u.check();
    let s = wait_for(&u, |p| p == Phase::Failed).await;
    assert_eq!(s.error, "No release has been published yet.");
}

#[tokio::test]
async fn rate_limit_and_offline_messages() {
    let (url, _f) = start(Fake { version: "1.3.1".into(), limit: true, ..Default::default() }).await;
    let d = tempfile::tempdir().unwrap();
    let u = updater(d.path(), "1.3.0", &url);
    u.check();
    let s = wait_for(&u, |p| p == Phase::Failed).await;
    assert!(s.error.contains("limiting update checks") && s.error.contains("10 minutes"), "{}", s.error);

    // Nothing listens on port 1: offline.
    let u = updater(d.path(), "1.3.0", "http://127.0.0.1:1/releases/latest");
    u.check();
    let s = wait_for(&u, |p| p == Phase::Failed).await;
    assert_eq!(s.error, "Can't reach GitHub. Check your internet connection and try again.");
    // Retry repeats the check (still offline), it does not panic or hang.
    u.retry(d.path().join("dl"));
    wait_for(&u, |p| p == Phase::Failed).await;
}

#[tokio::test]
async fn download_verifies_checksum() {
    let good = payload(200_000);
    let (url, fake) = start(Fake { version: "1.3.1".into(), good: good.clone(), served: good.clone(), ..Default::default() }).await;
    let d = tempfile::tempdir().unwrap();
    let dl = d.path().join("dl");
    let u = updater(d.path(), "1.3.0", &url);
    u.check();
    wait_for(&u, |p| p == Phase::Available).await;

    u.download(dl.clone());
    let s = wait_for(&u, |p| p == Phase::Ready).await;
    let f = s.file.unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), good);
    assert_eq!(f.file_name().unwrap(), "superchat-1.3.1-android.apk");
    assert!(!dl.join("superchat-1.3.1-android.apk.part").exists());
    assert_eq!(s.done, s.total);

    // Tampered bytes: refused, nothing left behind, can be retried.
    fake.lock().unwrap().served = payload(200_001);
    let u = updater(d.path(), "1.3.0", &url);
    std::fs::remove_dir_all(&dl).unwrap();
    u.download(dl.clone());
    let s = wait_for(&u, |p| p == Phase::Failed).await;
    assert!(s.error.contains("does not match its checksum"), "{}", s.error);
    assert!(std::fs::read_dir(&dl).unwrap().next().is_none(), "no file may stay after a bad checksum");
    fake.lock().unwrap().served = good.clone();
    u.retry(dl.clone());
    wait_for(&u, |p| p == Phase::Ready).await;

    // No checksum asset: refuse.
    {
        let mut f = fake.lock().unwrap();
        f.no_sha = true;
    }
    let u = updater(d.path(), "1.3.0", &url);
    u.check();
    wait_for(&u, |p| p == Phase::Available).await;
    std::fs::remove_dir_all(&dl).unwrap();
    u.download(dl.clone());
    let s = wait_for(&u, |p| p == Phase::Failed).await;
    assert!(s.error.contains("no checksum"), "{}", s.error);
    assert!(!dl.exists() || std::fs::read_dir(&dl).unwrap().next().is_none());
}

#[tokio::test]
async fn cancel_mid_download_leaves_nothing() {
    let good = payload(400_000);
    let (url, _f) = start(Fake { version: "1.3.1".into(), good: good.clone(), served: good, slow: true, ..Default::default() }).await;
    let d = tempfile::tempdir().unwrap();
    let dl = d.path().join("dl");
    let u = updater(d.path(), "1.3.0", &url);
    u.check();
    wait_for(&u, |p| p == Phase::Available).await;
    u.download(dl.clone());
    let s = wait_for(&u, |p| p == Phase::Downloading && u.status().done > 0).await;
    assert!(s.total > 0);
    u.cancel();
    let s = wait_for(&u, |p| p == Phase::Available).await;
    assert_eq!(s.error, "");
    assert!(std::fs::read_dir(&dl).unwrap().next().is_none());
}

#[tokio::test]
async fn skip_remind_and_schedule() {
    let good = payload(1000);
    let (url, fake) = start(Fake { version: "1.3.1".into(), good: good.clone(), served: good, ..Default::default() }).await;
    let d = tempfile::tempdir().unwrap();
    let u = updater(d.path(), "1.3.0", &url);
    let now = 1_800_000_000_000i64;

    // Automatic checks off: nothing happens.
    u.auto_tick(false, now);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.lock().unwrap().hits, 0);
    // On: checks at startup.
    u.auto_tick(true, now);
    wait_for(&u, |p| p == Phase::Available).await;
    assert_eq!(fake.lock().unwrap().hits, 1);
    // Not again until 24 h passed (the clock the updater itself uses is the real one; pretend with a past last check).
    u.auto_tick(true, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 + 3_600_000);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.lock().unwrap().hits, 1);
    u.auto_tick(true, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 + 25 * 3_600_000);
    wait_for(&u, |p| p == Phase::Available).await;
    for _ in 0..40 {
        if fake.lock().unwrap().hits == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(fake.lock().unwrap().hits, 2);

    // Remind me later hides the banner for a day; skip hides this version; both survive a restart.
    let s = u.status();
    assert!(s.show_banner(now));
    u.remind_later(now);
    assert!(!u.status().show_banner(now + 1000));
    assert!(u.status().show_banner(now + update::REMIND_LATER_MS));
    u.skip_version();
    assert!(!u.status().show_banner(now + update::REMIND_LATER_MS + 1));
    assert!(u.status().has_update(), "skipping only hides the banner; the About page still offers it");
    let u2 = updater(d.path(), "1.3.0", &url);
    assert!(!u2.status().show_banner(now + update::REMIND_LATER_MS + 1));
    assert_eq!(u2.status().skipped, "1.3.1");
    // A newer release than the skipped one shows again.
    fake.lock().unwrap().version = "1.3.2".into();
    u2.check();
    wait_for(&u2, |p| p == Phase::Available).await;
    assert!(u2.status().show_banner(now + update::REMIND_LATER_MS + 1));
}

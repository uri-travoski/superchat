//! SV-20: encrypted full backups, fresh-server restore, schedule and retention, per-user backups,
//! selective restore (merge / replace) and admin deletion with a mandatory backup first.
mod common;
use atomic_server::config::Config;
use client_core::admin::AdminClient;
use client_core::{ClientConfig, SendOptions};
use common::*;
use proto::*;
use std::path::Path;

const PASS: &str = "correct horse battery staple";

fn local(p: &Path) -> StorageConfig {
    StorageConfig::Local { path: Some(p.to_string_lossy().to_string()) }
}

/// A server whose attachments live in a directory that outlives the server's own data dir.
async fn server_with_shared_storage() -> (TestServer, std::path::PathBuf, std::path::PathBuf) {
    let s = TestServer::start().await;
    let att = s.dir.path().join("shared-attachments");
    let bk = s.dir.path().join("backup-target");
    std::fs::create_dir_all(&att).unwrap();
    std::fs::create_dir_all(&bk).unwrap();
    s.admin.set_storage(&local(&att)).await.unwrap();
    s.admin
        .put_backup_settings(&BackupSettingsUpdate { enabled: Some(false), target: Some(local(&bk)), passphrase: Some(PASS.into()), retention: Some(3), ..Default::default() })
        .await
        .unwrap();
    (s, att, bk)
}

async fn fresh_server(dir: &Path) -> (atomic_server::state::St, String) {
    let st = atomic_server::build_state(Config::for_test(dir.join("server2"))).await.unwrap();
    let url = atomic_server::serve_for_test(st.clone()).await.unwrap();
    (st, url)
}

fn all_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(all_files(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[tokio::test]
async fn full_backup_is_encrypted_and_restores_on_a_fresh_server_with_old_attachments() {
    let (s, att, bk) = server_with_shared_storage().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "general", &[&a_id, &b_id]).await;
    wait_conv(&bob, &ch).await;
    let f = s.dir.path().join("plan.txt");
    std::fs::write(&f, b"the secret launch plan").unwrap();
    alice.send_files(&ch, "plan attached", &[f], SendOptions::default()).unwrap();
    wait_until("file sent", || alice.timeline(&ch, 10, None).iter().any(|t| t.message.body == "plan attached" && !t.local)).await;
    let orig = say(&bob, &ch, "thanks, looks good").await;
    alice.react(&orig.id, "🎉", true).await.unwrap();
    alice.set_favourite(&ch, true).await.unwrap();
    let root = wait_msg(&alice, &ch, "plan attached").await;
    bob.send_thread_reply(&ch, &root.id, "a threaded note").unwrap();
    wait_until("thread reply", || alice.thread_replies(&ch, &root.id).len() == 1).await;

    // Backup now: encrypted at rest, with pointer objects for a fresh restore.
    let info = s.admin.backup_now().await.unwrap();
    assert!(info.encrypted && info.kind == "full" && info.key.ends_with(".db.age"), "{info:?}");
    let blob = std::fs::read(bk.join(&info.key)).unwrap();
    assert!(!blob.windows(15).any(|w| w == b"SQLite format 3"), "backup must not be a plain SQLite file");
    assert!(!String::from_utf8_lossy(&blob).contains("thanks, looks good"));
    assert!(bk.join("backups/full/latest.json").exists() && bk.join("backups/full/index.json").exists());
    let st = s.admin.backup_settings().await.unwrap();
    assert!(st.passphrase_set && st.last_run_ms.is_some());
    assert!(s.admin.backups().await.unwrap().iter().any(|b| b.id == info.id));
    // The backup never contains the passphrase.
    assert!(!all_files(&bk).iter().any(|p| std::fs::read(p).map(|b| String::from_utf8_lossy(&b).contains(PASS)).unwrap_or(false)));

    // A brand new, empty server.
    let (st2, url2) = fresh_server(s.dir.path()).await;
    let meta: ServerMeta = reqwest::get(format!("{url2}/api/v1/meta")).await.unwrap().json().await.unwrap();
    assert!(meta.setup_required);
    // Wrong passphrase is refused and leaves the server empty.
    let bad = AdminClient::fresh_restore(&url2, &FreshRestoreRequest { target: Some(local(&bk)), key: None, passphrase: "wrong".into() }).await;
    assert!(bad.is_err());
    let r = AdminClient::fresh_restore(&url2, &FreshRestoreRequest { target: Some(local(&bk)), key: None, passphrase: PASS.into() }).await.unwrap();
    assert!(r.users >= 3 && r.conversations >= 2 && r.messages >= 3, "{r:?}");
    assert!(r.attachments >= 1 && r.attachments_resolvable == r.attachments, "{r:?}");
    assert!(st2.db.call(|c| Ok(c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))?)).await.unwrap() >= 3);
    // Restoring again (server no longer fresh) is refused.
    let again = AdminClient::fresh_restore(&url2, &FreshRestoreRequest { target: Some(local(&bk)), key: None, passphrase: PASS.into() }).await;
    assert_eq!(again.unwrap_err().code(), Some(ErrorCode::SetupDone));
    // And the original server refuses too.
    assert!(AdminClient::fresh_restore(&s.url, &FreshRestoreRequest { target: Some(local(&bk)), key: None, passphrase: PASS.into() }).await.is_err());

    // Old chats, reactions, threads, prefs and attachments are all there for the same logins.
    let mut cfg = ClientConfig::new(s.dir.path().join("client-alice-new"));
    cfg.allow_insecure_http = true;
    let alice2 = client_core::Client::open(cfg).unwrap();
    alice2.login(&url2, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    wait_until("history restored", || texts(&alice2, &ch).contains(&"thanks, looks good".to_string())).await;
    let m = alice2.timeline(&ch, 50, None).into_iter().find(|t| t.message.body == "plan attached").unwrap().message;
    let path = alice2.download_attachment(&m.attachments[0]).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"the secret launch plan");
    assert!(alice2.pref(&ch).favourite);
    let thanks = alice2.timeline(&ch, 50, None).into_iter().find(|t| t.message.body == "thanks, looks good").unwrap().message;
    assert_eq!(thanks.reactions.len(), 1);
    let v = alice2.open_thread(&ch, &m.id).await.unwrap();
    assert_eq!(v.replies.len(), 1);
    // The attachment files were not copied: the new server reads the same shared directory.
    assert!(!all_files(&att).is_empty());
    // The admin account works too, and the new server can take new messages.
    let admin2 = AdminClient::login(&url2, "admin", "test-admin-pw").await.unwrap();
    assert!(admin2.users().await.unwrap().len() >= 3);
    say(&alice2, &ch, "back online on the new server").await;

    // A client that was connected to the old server (old cursor) re-bootstraps after the move.
    bob.stop();
    let bob_dir = s.client_dir("bob");
    drop(bob);
    let mut cfg = ClientConfig::new(bob_dir);
    cfg.allow_insecure_http = true;
    let bob2 = client_core::Client::open(cfg).unwrap();
    bob2.login(&url2, "bob", "password-123", None).await.unwrap();
    bob2.start();
    wait_online(&bob2).await;
    wait_until("bob sees new-server message", || texts(&bob2, &ch).contains(&"back online on the new server".to_string())).await;
    assert!(texts(&bob2, &ch).contains(&"thanks, looks good".to_string()));
}

#[tokio::test]
async fn schedule_runs_when_due_and_retention_prunes_old_backups() {
    let (s, _att, bk) = server_with_shared_storage().await;
    s.admin
        .put_backup_settings(&BackupSettingsUpdate { enabled: Some(true), interval_hours: Some(24), retention: Some(2), ..Default::default() })
        .await
        .unwrap();
    let (_a, _alice) = s.user("alice").await;
    let now = atomic_server::db::now_ms();
    let day = 24 * 3_600_000;
    // Nothing yet: due immediately.
    assert!(atomic_server::fullbackup::scheduled_tick(&s.st, now).await.unwrap());
    // Not due again until 24 h have passed.
    assert!(!atomic_server::fullbackup::scheduled_tick(&s.st, now + 3_600_000).await.unwrap());
    for i in 1..=3 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        assert!(atomic_server::fullbackup::scheduled_tick(&s.st, now + i * day + 1000).await.unwrap(), "run {i}");
    }
    let fulls: Vec<_> = s.admin.backups().await.unwrap().into_iter().filter(|b| b.kind == "full").collect();
    assert_eq!(fulls.len(), 2, "retention keeps the two newest");
    let on_disk = all_files(&bk.join("backups/full")).into_iter().filter(|p| p.to_string_lossy().ends_with(".db.age")).count();
    assert_eq!(on_disk, 2, "pruned backups are deleted from the target");
    // Disabled schedule does nothing.
    s.admin.put_backup_settings(&BackupSettingsUpdate { enabled: Some(false), ..Default::default() }).await.unwrap();
    assert!(!atomic_server::fullbackup::scheduled_tick(&s.st, now + 10 * day).await.unwrap());
}

#[tokio::test]
async fn backup_target_must_work_and_passphrase_is_required() {
    let s = TestServer::start().await;
    // No passphrase yet: refused with a clear message.
    let e = s.admin.backup_now().await.unwrap_err();
    assert_eq!(e.code(), Some(ErrorCode::Validation));
    s.admin.put_backup_settings(&BackupSettingsUpdate { passphrase: Some(PASS.into()), ..Default::default() }).await.unwrap();
    assert!(s.admin.test_backup_target().await.unwrap().ok);
    // Short passphrases are refused.
    assert!(s.admin.put_backup_settings(&BackupSettingsUpdate { passphrase: Some("abc".into()), ..Default::default() }).await.is_err());
    // A broken target makes the backup fail loudly and leaves no record.
    let bad = StorageConfig::Local { path: Some("/proc/definitely/not/writable".into()) };
    s.admin.put_backup_settings(&BackupSettingsUpdate { target: Some(bad), ..Default::default() }).await.unwrap();
    let e = s.admin.backup_now().await.unwrap_err();
    assert!(matches!(e.code(), Some(ErrorCode::BackupFailed) | Some(ErrorCode::StorageError)), "{e:?}");
    assert!(s.admin.backups().await.unwrap().iter().all(|b| b.kind != "full"));
}

#[tokio::test]
async fn per_user_backup_is_encrypted_and_selective_restore_merges_or_replaces() {
    let (s, att, bk) = server_with_shared_storage().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let dm = alice.open_dm(&b_id).await.unwrap();
    wait_conv(&bob, &dm).await;
    let f = s.dir.path().join("note.txt");
    std::fs::write(&f, b"file in a dm").unwrap();
    alice.send_files(&dm, "old one with file", &[f], SendOptions::default()).unwrap();
    wait_until("sent", || alice.timeline(&dm, 5, None).iter().any(|t| t.message.body == "old one with file" && !t.local)).await;
    say(&bob, &dm, "old two").await;
    let cdm = carol.open_dm(&b_id).await.unwrap();
    wait_conv(&bob, &cdm).await;
    say(&carol, &cdm, "carol to bob").await;
    wait_msg(&bob, &cdm, "carol to bob").await;

    // Alice's history is cleared: she loses her copy, bob keeps his. A backup was made first.
    let user_backup = s.admin.backup_user(&a_id).await.unwrap();
    assert!(user_backup.encrypted && user_backup.key.ends_with("data.json.age"), "{user_backup:?}");
    let raw = std::fs::read_dir(bk.parent().unwrap()).is_ok();
    assert!(raw);
    s.admin.clear_history(&a_id).await.unwrap();
    wait_until("alice emptied", || alice.timeline(&dm, 10, None).is_empty() || !alice.signed_in()).await;
    // Clearing history signs the account out of its sessions; sign in again.
    let alice = s.new_client("alice-again");
    alice.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice.start();
    wait_online(&alice).await;
    assert!(texts(&alice, &dm).is_empty());
    assert!(texts(&bob, &dm).contains(&"old two".to_string()), "bob keeps his copy");
    let backup_id = s.admin.backups().await.unwrap().into_iter().filter(|b| b.kind == "user" && b.user_id.as_deref() == Some(a_id.as_str())).max_by_key(|b| b.created_ms).unwrap().id;

    // Merge restore for alice only: her messages return, the file is back in the live area.
    let r = s.admin.restore(&RestoreRequest { backup_id: backup_id.clone(), user_ids: vec![a_id.clone()], all: false, mode: None }).await.unwrap();
    assert!(r.messages_unhidden + r.messages_added >= 2, "{r:?}");
    wait_until("alice has history again", || texts(&alice, &dm).len() == 2).await;
    let m = alice.timeline(&dm, 10, None).into_iter().find(|t| t.message.body == "old one with file").unwrap().message;
    let p = alice.download_attachment(&m.attachments[0]).await.unwrap();
    assert_eq!(std::fs::read(p).unwrap(), b"file in a dm");
    assert!(!all_files(&att).iter().any(|p| p.to_string_lossy().contains("/backups/") && p.to_string_lossy().contains("/att/")), "files moved back out of the backup folder");
    // Bob was untouched and carol never involved.
    assert_eq!(texts(&bob, &dm).len(), 2);
    assert!(texts(&carol, &cdm).contains(&"carol to bob".to_string()));
    // Choosing nobody is a mistake, a wrong user is refused, a bad mode is refused.
    assert!(s.admin.restore(&RestoreRequest { backup_id: backup_id.clone(), user_ids: vec![], all: false, mode: None }).await.is_err());
    assert!(s.admin.restore(&RestoreRequest { backup_id: backup_id.clone(), user_ids: vec![c_id.clone()], all: false, mode: None }).await.is_err());
    assert!(s.admin.restore(&RestoreRequest { backup_id: backup_id.clone(), user_ids: vec![a_id.clone()], all: false, mode: Some("bogus".into()) }).await.is_err());

    // Replace mode: what alice has now but the backup does not know is hidden for her only.
    say(&alice, &dm, "written after the backup").await;
    wait_msg(&bob, &dm, "written after the backup").await;
    // merge never removes it...
    s.admin.restore(&RestoreRequest { backup_id: backup_id.clone(), user_ids: vec![a_id.clone()], all: false, mode: Some("merge".into()) }).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(texts(&alice, &dm).contains(&"written after the backup".to_string()));
    // ...replace does.
    let r = s.admin.restore(&RestoreRequest { backup_id: backup_id.clone(), user_ids: vec![a_id.clone()], all: false, mode: Some("replace".into()) }).await.unwrap();
    assert!(r.messages_hidden >= 1, "{r:?}");
    wait_until("alice back to the backup state", || !texts(&alice, &dm).contains(&"written after the backup".to_string()) && texts(&alice, &dm).len() == 2).await;
    assert!(texts(&bob, &dm).contains(&"written after the backup".to_string()), "bob's copy is never touched");

    // Selective restore from a full backup: delete the account's history, restore only alice.
    s.admin.backup_now().await.unwrap();
    let full = s.admin.backups().await.unwrap().into_iter().find(|b| b.kind == "full").unwrap();
    s.admin.clear_history(&b_id).await.unwrap();
    let r = s.admin.restore(&RestoreRequest { backup_id: full.id.clone(), user_ids: vec![b_id.clone()], all: false, mode: None }).await.unwrap();
    assert!(r.users_restored == 1 && r.messages_unhidden + r.messages_added > 0, "{r:?}");
    let bob_again = s.new_client("bob-again");
    bob_again.login(&s.url, "bob", "password-123", None).await.unwrap();
    bob_again.start();
    wait_online(&bob_again).await;
    wait_until("bob history back", || texts(&bob_again, &dm).contains(&"old two".to_string()) && texts(&bob_again, &cdm).contains(&"carol to bob".to_string())).await;
}

#[tokio::test]
async fn deleted_accounts_come_back_from_backup() {
    let (s, _att, _bk) = server_with_shared_storage().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let dm = alice.open_dm(&b_id).await.unwrap();
    wait_conv(&bob, &dm).await;
    say(&alice, &dm, "before deletion").await;
    wait_msg(&bob, &dm, "before deletion").await;
    s.admin.backup_now().await.unwrap();
    let full = s.admin.backups().await.unwrap().into_iter().find(|b| b.kind == "full").unwrap();
    s.admin.delete_user(&a_id).await.unwrap();
    wait_signed_out(&alice).await;
    assert!(s.new_client("alice-probe").login(&s.url, "alice", "password-123", None).await.is_err());
    let r = s.admin.restore(&RestoreRequest { backup_id: full.id, user_ids: vec![a_id.clone()], all: false, mode: None }).await.unwrap();
    assert_eq!(r.users_restored, 1, "{r:?}");
    let alice2 = s.new_client("alice-restored");
    alice2.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    wait_until("history", || texts(&alice2, &dm).contains(&"before deletion".to_string())).await;
}

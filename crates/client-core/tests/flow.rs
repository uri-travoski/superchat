mod common;
use common::*;
use client_core::{ClientError, SendOptions, SendState};
use proto::*;

fn api_code(e: &ClientError) -> Option<ErrorCode> {
    e.code()
}

#[tokio::test]
async fn login_dm_and_realtime_delivery() {
    let s = TestServer::start().await;
    let (_a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;

    let conv = alice.open_dm(&b_id).await.unwrap();
    alice.send_text(&conv, "hello bob", SendOptions::default()).unwrap();
    wait_until("bob receives", || texts(&bob, &conv).contains(&"hello bob".to_string())).await;
    // unread count for bob, none for alice
    wait_until("bob unread", || bob.conversations().iter().any(|c| c.conversation.id == conv && c.unread == 1)).await;
    assert_eq!(alice.conversations().iter().find(|c| c.conversation.id == conv).unwrap().unread, 0);
    // delivery state reaches the sender
    wait_until("delivered", || alice.timeline(&conv, 10, None).iter().any(|t| t.state == SendState::Delivered)).await;
    // read state syncs
    bob.mark_read(&conv).await.unwrap();
    assert_eq!(bob.conversations().iter().find(|c| c.conversation.id == conv).unwrap().unread, 0);
    // the DM is titled with the other person's display name
    let cs = bob.conversations().into_iter().find(|c| c.conversation.id == conv).unwrap();
    assert_eq!(bob.conversation_title(&cs), "Alice");
}

#[tokio::test]
async fn notes_to_self_exists() {
    let s = TestServer::start().await;
    let (_id, alice) = s.user("alice").await;
    let me = alice.self_conversation().expect("self conversation");
    alice.send_text(&me.conversation.id, "remember the milk", SendOptions::default()).unwrap();
    wait_until("self msg", || texts(&alice, &me.conversation.id).contains(&"remember the milk".to_string())).await;
    // No delivery indicator for notes to self.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(alice.timeline(&me.conversation.id, 10, None).iter().all(|t| t.state != SendState::Delivered));
}

#[tokio::test]
async fn edit_rule_dm_and_channel() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();

    alice.send_text(&conv, "first", SendOptions::default()).unwrap();
    wait_until("sent", || texts(&alice, &conv).len() == 1 && alice.timeline(&conv, 5, None)[0].state != SendState::Sending).await;
    let m = alice.timeline(&conv, 5, None)[0].message.clone();
    assert!(alice.can_edit(&m));
    let edited = alice.edit_message(&m.id, "first (fixed)").await.unwrap();
    assert_eq!(edited.version, 2);
    assert!(edited.is_edited());
    wait_until("bob sees edit", || texts(&bob, &conv) == vec!["first (fixed)".to_string()]).await;

    // Bob replies; Alice's earlier message can no longer be edited (server decides).
    wait_until("bob has msg", || bob.timeline(&conv, 5, None).len() == 1).await;
    bob.send_text(&conv, "reply", SendOptions::default()).unwrap();
    wait_until("alice gets reply", || texts(&alice, &conv).len() == 2).await;
    assert!(!alice.can_edit(&m), "client mirrors the rule and hides Edit");
    let err = alice.edit_message(&m.id, "too late").await.unwrap_err();
    assert_eq!(api_code(&err), Some(ErrorCode::EditWindowClosed));
    // Bob's reply is still editable: nobody replied after it.
    let reply = bob.timeline(&conv, 5, None)[1].message.clone();
    assert!(bob.edit_message(&reply.id, "reply!").await.is_ok());

    // Channels: editable forever, even after replies.
    let ch = s.admin.create_channel("project", &[a_id.clone(), b_id.clone()]).await.unwrap();
    let cid = ch.conversation.id.clone();
    wait_until("alice has channel", || alice.conversations().iter().any(|c| c.conversation.id == cid)).await;
    wait_until("bob has channel", || bob.conversations().iter().any(|c| c.conversation.id == cid)).await;
    alice.send_text(&cid, "channel one", SendOptions::default()).unwrap();
    wait_until("bob sees", || texts(&bob, &cid).len() == 1).await;
    bob.send_text(&cid, "channel two", SendOptions::default()).unwrap();
    wait_until("alice sees", || texts(&alice, &cid).len() == 2).await;
    let first = alice.timeline(&cid, 5, None)[0].message.clone();
    assert!(alice.can_edit(&first));
    assert!(alice.edit_message(&first.id, "channel one (edited)").await.is_ok());

    // Only the author may edit.
    let err = bob.edit_message(&first.id, "hijack").await.unwrap_err();
    assert_eq!(api_code(&err), Some(ErrorCode::Forbidden));
}

#[tokio::test]
async fn delete_leaves_tombstone_for_everyone() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    alice.send_text(&conv, "oops secret", SendOptions::default()).unwrap();
    wait_until("bob sees", || texts(&bob, &conv).len() == 1).await;
    let m = alice.timeline(&conv, 5, None)[0].message.clone();
    alice.delete_message(&m.id).await.unwrap();
    wait_until("bob tombstone", || bob.timeline(&conv, 5, None).first().map(|t| t.message.deleted).unwrap_or(false)).await;
    let t = &bob.timeline(&conv, 5, None)[0].message;
    assert!(t.body.is_empty());
    // Bob cannot delete Alice's message.
    let fresh = alice.send_text(&conv, "again", SendOptions::default()).unwrap();
    let _ = fresh;
    wait_until("bob sees again", || bob.timeline(&conv, 5, None).len() == 2).await;
    let m2 = bob.timeline(&conv, 5, None)[1].message.clone();
    assert_eq!(bob.delete_message(&m2.id).await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    // Search no longer finds deleted text.
    assert!(bob.search("secret").await.unwrap().hits.is_empty());
}

#[tokio::test]
async fn reactions_sync() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    alice.send_text(&conv, "react to me", SendOptions::default()).unwrap();
    wait_until("bob sees", || texts(&bob, &conv).len() == 1).await;
    let id = bob.timeline(&conv, 5, None)[0].message.id.clone();
    bob.react(&id, "👍", true).await.unwrap();
    wait_until("alice sees reaction", || alice.timeline(&conv, 5, None)[0].message.reactions.iter().any(|r| r.emoji == "👍" && r.user_ids.len() == 1)).await;
    bob.react(&id, "👍", false).await.unwrap();
    wait_until("reaction gone", || alice.timeline(&conv, 5, None)[0].message.reactions.is_empty()).await;
}

#[tokio::test]
async fn sends_are_idempotent() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let me = alice.self_conversation().unwrap().conversation.id;
    let api = alice.api().unwrap();
    let req = SendMessage { client_id: "same-id".into(), body: "once".into(), thread_root: None, reply_to: None, mentions: vec![], attachment_ids: vec![] };
    let m1: Message = api.post(&format!("/conversations/{me}/messages"), &req).await.unwrap();
    let m2: Message = api.post(&format!("/conversations/{me}/messages"), &req).await.unwrap();
    assert_eq!(m1.id, m2.id);
    assert_eq!(m1.seq, m2.seq);
    let h: HistoryResponse = api.get(&format!("/conversations/{me}/messages")).await.unwrap();
    assert_eq!(h.messages.len(), 1);
}

#[tokio::test]
async fn private_channels_and_admin_only_creation() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (_b_id, bob) = s.user("bob").await;
    // Members cannot create channels or group DMs.
    let api = alice.api().unwrap();
    let err = api
        .post::<ConversationSummary, _>("/conversations", &CreateConversation { kind: ConversationKind::Channel, name: "nope".into(), topic: None, members: vec![] })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::Forbidden));
    // No public channels exist: only channel and group_dm kinds, both members-only.
    let ch = s.admin.create_channel("secret-project", std::slice::from_ref(&a_id)).await.unwrap();
    let cid = ch.conversation.id;
    wait_until("alice has it", || alice.conversations().iter().any(|c| c.conversation.id == cid)).await;
    // Bob never learns about it, and direct access is hidden (404, not 403).
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(bob.conversations().iter().all(|c| c.conversation.id != cid));
    let r = bob.api().unwrap().get::<ConversationSummary>(&format!("/conversations/{cid}")).await.unwrap_err();
    assert_eq!(r.code(), Some(ErrorCode::NotFound));
    let r = bob.api().unwrap().get::<HistoryResponse>(&format!("/conversations/{cid}/messages")).await.unwrap_err();
    assert_eq!(r.code(), Some(ErrorCode::NotFound));
    // Adding Bob makes it appear automatically (M-01).
    let b = s.admin.users().await.unwrap().into_iter().find(|u| u.user.username == "bob").unwrap();
    s.admin.add_members(&cid, std::slice::from_ref(&b.user.id)).await.unwrap();
    wait_until("bob has it", || bob.conversations().iter().any(|c| c.conversation.id == cid)).await;
    // Group DMs need at least two members.
    assert!(s.admin.create_group("g", &[a_id]).await.is_err());
}

#[tokio::test]
async fn search_is_isolated_between_users() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let ab = alice.open_dm(&b_id).await.unwrap();
    alice.send_text(&ab, "the zebra plan is confidential", SendOptions::default()).unwrap();
    let ac = alice.open_dm(&c_id).await.unwrap();
    alice.send_text(&ac, "carol gets zebra stripes", SendOptions::default()).unwrap();
    wait_until("bob has msg", || texts(&bob, &ab).len() == 1).await;
    wait_until("carol has msg", || texts(&carol, &ac).len() == 1).await;
    let ra = alice.search("zebra").await.unwrap();
    assert_eq!(ra.hits.len(), 2);
    let rb = bob.search("zebra").await.unwrap();
    assert_eq!(rb.hits.len(), 1);
    assert!(rb.hits[0].snippet.contains("[[zebra]]"));
    let rc = carol.search("zebra").await.unwrap();
    assert_eq!(rc.hits.len(), 1);
    assert_eq!(rc.hits[0].message.conv_id, ac);
    // Bob cannot see Alice-Carol content even with a filter on that conversation.
    let r: SearchResponse = bob.api().unwrap().get(&format!("/search?q=stripes&in={ac}")).await.unwrap();
    assert!(r.hits.is_empty());
    // Local offline search over the cache.
    assert_eq!(bob.search_local("zebra").len(), 1);
}

#[tokio::test]
async fn upload_download_and_size_limit() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    let dir = s.dir.path().join("files");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("notes.txt");
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&f, &data).unwrap();
    alice.send_files(&conv, "see attached", std::slice::from_ref(&f), SendOptions::default()).unwrap();
    wait_until("bob gets file msg", || bob.timeline(&conv, 5, None).iter().any(|t| !t.message.attachments.is_empty())).await;
    let att = bob.timeline(&conv, 5, None)[0].message.attachments[0].clone();
    assert_eq!(att.name, "notes.txt");
    assert_eq!(att.size, 300_000);
    let p = bob.download_attachment(&att).await.unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), data);
    // Non-members cannot fetch it.
    let _ = c_id;
    let err = carol.download_attachment(&att).await.unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::NotFound));
    // Over the limit: the server refuses with file_too_large (limit is 5 MB in tests).
    let big = dir.join("big.bin");
    std::fs::write(&big, vec![7u8; 6 * 1024 * 1024]).unwrap();
    let ct = alice.send_files(&conv, "", &[big], SendOptions::default());
    // Client knows the limit once the server meta is known; either way it never gets stored.
    if let Ok(cid) = ct {
        wait_until("big upload failed", || alice.timeline(&conv, 10, None).iter().any(|t| t.state == SendState::Failed)).await;
        alice.cancel_send(&cid).unwrap();
    } else {
        assert_eq!(ct.unwrap_err().code(), Some(ErrorCode::FileTooLarge));
    }
    // Range requests work (video seeking).
    let api = bob.api().unwrap();
    let url = api.download_url(&att.id);
    let resp = api.exec(|c| c.get(&url).header("Range", "bytes=10-19")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 206);
    assert_eq!(resp.bytes().await.unwrap().to_vec(), data[10..20].to_vec());
}

#[tokio::test]
async fn sync_after_offline_catches_up_by_cursor() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    alice.send_text(&conv, "m0", SendOptions::default()).unwrap();
    wait_until("bob m0", || texts(&bob, &conv).len() == 1).await;
    let cursor_before = bob.store().cursor();
    bob.stop();
    for i in 1..=5 {
        alice.send_text(&conv, &format!("m{i}"), SendOptions::default()).unwrap();
    }
    wait_until("alice sent all", || alice.timeline(&conv, 10, None).iter().filter(|t| t.state != SendState::Sending).count() == 6).await;
    assert_eq!(texts(&bob, &conv).len(), 1, "bob is offline and has not seen them");
    bob.start();
    wait_until("bob caught up", || texts(&bob, &conv).len() == 6).await;
    assert!(bob.store().cursor() > cursor_before);
    assert_eq!(texts(&bob, &conv), vec!["m0", "m1", "m2", "m3", "m4", "m5"]);
}

#[tokio::test]
async fn outbox_survives_being_offline() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let me = alice.self_conversation().unwrap().conversation.id;
    // Take the server away from the client by pointing a second client at a dead port.
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    alice.stop();
    alice.send_text(&conv, "queued while offline", SendOptions::default()).unwrap();
    let item = alice.timeline(&conv, 5, None);
    assert_eq!(item[0].state, SendState::Sending);
    assert!(item[0].local);
    assert!(texts(&bob, &conv).is_empty());
    let _ = me;
    alice.start();
    wait_until("delivered after reconnect", || texts(&bob, &conv) == vec!["queued while offline".to_string()]).await;
    wait_until("alice no longer pending", || alice.timeline(&conv, 5, None).iter().all(|t| t.state != SendState::Sending)).await;
    assert_eq!(alice.timeline(&conv, 5, None).len(), 1, "no duplicate after the retry");
}

#[tokio::test]
async fn disabled_account_is_kicked_and_wiped() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    alice.send_text(&conv, "before disable", SendOptions::default()).unwrap();
    wait_until("bob has it", || texts(&bob, &conv).len() == 1).await;
    assert!(!bob.conversations().is_empty());

    let waiter = {
        let b = bob.clone();
        tokio::spawn(async move { wait_signed_out(&b).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    s.admin.set_disabled(&b_id, true).await.unwrap();
    let (code, msg) = tokio::time::timeout(std::time::Duration::from_secs(10), waiter).await.expect("kicked immediately").unwrap();
    assert_eq!(code, Some(ErrorCode::AccountDisabled));
    assert!(msg.contains("disabled"));
    // X-05: nothing left on the device.
    assert!(!bob.signed_in());
    assert!(bob.conversations().is_empty());
    assert!(bob.users().is_empty());
    assert!(bob.store().cursor() == 0);
    assert!(!bob.data_dir().join("secrets.json").exists() || !std::fs::read_to_string(bob.data_dir().join("secrets.json")).unwrap().contains("atomic-chat.session"));
    // Cannot sign in again while disabled.
    let c2 = s.new_client("bob");
    let err = c2.login(&s.url, "bob", "password-123", None).await.unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::AccountDisabled));
    // Re-enabled: signs in and the history comes back (cloud chats).
    s.admin.set_disabled(&b_id, false).await.unwrap();
    c2.login(&s.url, "bob", "password-123", None).await.unwrap();
    c2.start();
    wait_until("history is back", || texts(&c2, &conv) == vec!["before disable".to_string()]).await;
}

#[tokio::test]
async fn password_reset_forces_sign_out() {
    let s = TestServer::start().await;
    let (b_id, bob) = s.user("bob").await;
    let waiter = {
        let b = bob.clone();
        tokio::spawn(async move { wait_signed_out(&b).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    s.admin.reset_password(&b_id, Some("brand-new-pass")).await.unwrap();
    let (code, msg) = waiter.await.unwrap();
    assert_eq!(code, Some(ErrorCode::PasswordReset));
    assert!(msg.contains("reset your password"));
    let c = s.new_client("bob2");
    assert!(c.login(&s.url, "bob", "password-123", None).await.is_err());
    assert!(c.login(&s.url, "bob", "brand-new-pass", None).await.is_ok());
}

#[tokio::test]
async fn one_time_login_link() {
    let s = TestServer::start().await;
    let r = s.admin.create_user("newbie", "New Person", None).await.unwrap();
    let link = r.login_link.expect("link created when no password is given");
    assert!(link.link.starts_with("atomic-chat://login?server="));
    assert!(!r.user.password_set);
    let c = s.new_client("newbie");
    let me = c.login_with_link(&link.link, None, None).await.unwrap();
    assert_eq!(me.user.username, "newbie");
    assert!(!me.password_set);
    // The user then sets a password.
    c.change_password(None, "my-own-password").await.unwrap();
    // One-time: the same link cannot be used again.
    let c2 = s.new_client("newbie2");
    let err = c2.login_with_link(&link.link, None, None).await.unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::InvalidCredentials));
    // A bare token works when the server address is given separately.
    let l2 = s.admin.login_link(&r.user.user.id).await.unwrap();
    let c3 = s.new_client("newbie3");
    assert!(c3.login_with_link(&l2.token, Some(&s.url), None).await.is_ok());
    // The new password works.
    let c4 = s.new_client("newbie4");
    assert!(c4.login(&s.url, "newbie", "my-own-password", None).await.is_ok());
}

#[tokio::test]
async fn failed_logins_lock_out_temporarily() {
    let s = TestServer::start_with(|c| {
        c.lockout_threshold = 3;
        c.lockout_secs = 60;
    })
    .await;
    s.admin.create_user("victim", "Victim", Some("correct-horse")).await.unwrap();
    let c = s.new_client("x");
    for _ in 0..3 {
        let e = c.login(&s.url, "victim", "wrong", None).await.unwrap_err();
        assert_eq!(e.code(), Some(ErrorCode::InvalidCredentials));
    }
    let e = c.login(&s.url, "victim", "correct-horse", None).await.unwrap_err();
    assert_eq!(e.code(), Some(ErrorCode::LockedOut), "even the right password is refused during lockout");
}

#[tokio::test]
async fn clear_history_backs_up_hides_own_copy_only_and_moves_files() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    let f = s.dir.path().join("photo.png");
    std::fs::write(&f, vec![1u8; 4096]).unwrap();
    alice.send_files(&conv, "pic", &[f], SendOptions::default()).unwrap();
    alice.send_text(&conv, "and text", SendOptions::default()).unwrap();
    wait_until("bob has both", || bob.timeline(&conv, 5, None).len() == 2).await;
    let att = bob.timeline(&conv, 5, None)[0].message.attachments[0].clone();

    let waiter = {
        let a = alice.clone();
        tokio::spawn(async move { wait_signed_out(&a).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let r = s.admin.clear_history(&a_id).await.unwrap();
    assert_eq!(r.messages_hidden, 2);
    assert_eq!(r.attachments_moved, 0, "bob still sees the file, so it must not move");
    assert!(r.backup_key.contains("backups/users/"));
    let (code, _) = waiter.await.unwrap();
    assert_eq!(code, Some(ErrorCode::HistoryCleared));
    // Bob's copy is untouched, including the attachment.
    assert_eq!(texts(&bob, &conv), vec!["pic", "and text"]);
    assert!(bob.download_attachment(&att).await.is_ok());
    // The backup exists in storage and is listed.
    let backups = s.admin.backups().await.unwrap();
    assert_eq!(backups.len(), 1);
    assert!(s.st.storage().head(&backups[0].key).await.unwrap().is_some());
    let doc: serde_json::Value = serde_json::from_slice(&s.st.storage().get_all(&backups[0].key).await.unwrap()).unwrap();
    assert_eq!(doc["messages"].as_array().unwrap().len(), 2);
    assert_eq!(doc["user"]["username"], "alice");
    // Alice signs in again: an empty history, but her conversations are still there.
    let a2 = s.new_client("alice2");
    a2.login(&s.url, "alice", "password-123", None).await.unwrap();
    a2.start();
    wait_until("alice back", || !a2.conversations().is_empty()).await;
    assert!(texts(&a2, &conv).is_empty());
    assert!(!a2.has_more_before(&conv));
    // And she cannot fetch the file via the old message any more.
    assert_eq!(a2.download_attachment(&att).await.unwrap_err().code(), Some(ErrorCode::NotFound));
    // New messages flow normally.
    bob.send_text(&conv, "welcome back", SendOptions::default()).unwrap();
    wait_until("alice sees new", || texts(&a2, &conv) == vec!["welcome back".to_string()]).await;
}

#[tokio::test]
async fn clear_history_moves_orphaned_attachments_into_the_backup_folder() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let me = alice.self_conversation().unwrap().conversation.id;
    let f = s.dir.path().join("private.pdf");
    std::fs::write(&f, b"%PDF-1.4 secret").unwrap();
    alice.send_files(&me, "my file", &[f], SendOptions::default()).unwrap();
    wait_until("sent", || alice.timeline(&me, 5, None).iter().any(|t| !t.message.attachments.is_empty() && !t.local)).await;
    let att = alice.timeline(&me, 5, None)[0].message.attachments[0].clone();
    let key = format!("att/{}", att.id);
    assert!(s.st.storage().head(&key).await.unwrap().is_some());
    let r = s.admin.clear_history(&a_id).await.unwrap();
    assert_eq!(r.attachments_moved, 1);
    // Moved, not erased.
    assert!(s.st.storage().head(&key).await.unwrap().is_none());
    let moved = format!("{}/att/{}", r.backup_key.trim_end_matches("/data.json"), att.id);
    assert!(s.st.storage().head(&moved).await.unwrap().is_some());
    // The storage verification pass still finds it (it follows the moved key).
    let j = s.admin.verify_storage(true).await.unwrap();
    let j = s.admin.wait_job(&j.id).await.unwrap();
    assert_eq!(j.state, "done", "{j:?}");
}

#[tokio::test]
async fn clear_history_is_refused_when_the_backup_fails() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let me = alice.self_conversation().unwrap().conversation.id;
    alice.send_text(&me, "keep me", SendOptions::default()).unwrap();
    wait_until("sent", || alice.timeline(&me, 5, None).iter().any(|t| !t.local)).await;
    // Make the storage backend unusable: point local storage at a path that is a file.
    let bad = s.dir.path().join("not-a-dir");
    std::fs::write(&bad, b"x").unwrap();
    let broken = atomic_server::storage::build_storage(&StorageConfig::Local { path: Some(bad.to_string_lossy().to_string()) }, s.dir.path());
    // Building may already fail; in that case swap in a read-only dir instead.
    match broken {
        Ok(b) => *s.st.storage.write().unwrap() = b,
        Err(_) => {
            let ro = s.dir.path().join("ro");
            std::fs::create_dir_all(&ro).unwrap();
            let b = atomic_server::storage::build_storage(&StorageConfig::Local { path: Some(ro.to_string_lossy().to_string()) }, s.dir.path()).unwrap();
            std::fs::remove_dir_all(&ro).unwrap();
            std::fs::write(&ro, b"x").unwrap();
            *s.st.storage.write().unwrap() = b;
        }
    }
    let err = s.admin.clear_history(&a_id).await.unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::BackupFailed));
    // Nothing changed: still signed in, message still there, no backup recorded.
    assert!(alice.signed_in());
    assert_eq!(texts(&alice, &me), vec!["keep me"]);
    let h: HistoryResponse = alice.api().unwrap().get(&format!("/conversations/{me}/messages")).await.unwrap();
    assert_eq!(h.messages.len(), 1);
    assert!(s.admin.backups().await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_range_removes_only_selected_copy_by_default() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    alice.send_text(&conv, "one", SendOptions::default()).unwrap();
    wait_until("bob 1", || texts(&bob, &conv).len() == 1).await;
    let mid = alice.timeline(&conv, 5, None)[0].message.ts;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let cut = common_now();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    alice.send_text(&conv, "two", SendOptions::default()).unwrap();
    wait_until("bob 2", || texts(&bob, &conv).len() == 2).await;
    let r = s
        .admin
        .delete_range(&a_id, &DateRangeDeleteRequest { from_ms: mid - 10, to_ms: cut, before_ms: None, conversation_id: None, also_other_copies: false })
        .await
        .unwrap();
    assert_eq!(r.messages_hidden, 1);
    wait_until("alice lost one", || texts(&alice, &conv) == vec!["two"]).await;
    assert_eq!(texts(&bob, &conv), vec!["one", "two"], "other person's copy stays");
    // With the option set, the other copy goes too.
    let r = s
        .admin
        .delete_range(&a_id, &DateRangeDeleteRequest { from_ms: 0, to_ms: common_now() + 10_000, before_ms: None, conversation_id: Some(conv.clone()), also_other_copies: true })
        .await
        .unwrap();
    assert_eq!(r.messages_hidden, 1);
    wait_until("bob lost the second one", || texts(&bob, &conv) == vec!["one"]).await;
}

fn common_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[tokio::test]
async fn storage_move_between_backends_and_verify() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let me = alice.self_conversation().unwrap().conversation.id;
    let mut originals = vec![];
    for i in 0..3 {
        let f = s.dir.path().join(format!("f{i}.bin"));
        let data = vec![i as u8 + 1; 10_000 + i * 777];
        std::fs::write(&f, &data).unwrap();
        alice.send_files(&me, "", &[f], SendOptions::default()).unwrap();
        originals.push(data);
    }
    wait_until("3 uploaded", || alice.timeline(&me, 10, None).iter().filter(|t| !t.local && !t.message.attachments.is_empty()).count() == 3).await;
    let info = s.admin.storage().await.unwrap();
    assert_eq!(info.attachments, 3);
    let target_dir = s.dir.path().join("other-storage");
    let target = StorageConfig::Local { path: Some(target_dir.to_string_lossy().to_string()) };
    // Test button
    assert!(s.admin.test_storage(&target).await.unwrap().ok);
    // Changing the backend directly is refused while files exist; migration is the way.
    assert_eq!(s.admin.set_storage(&target).await.unwrap_err().code(), Some(ErrorCode::Conflict));
    let job = s.admin.migrate_storage(&target, true).await.unwrap();
    let job = s.admin.wait_job(&job.id).await.unwrap();
    assert_eq!(job.state, "done", "{job:?}");
    assert_eq!(job.done, 3);
    // Files physically moved.
    assert!(target_dir.join("att").exists());
    // The server still finds every attachment exactly as before.
    let tl = alice.timeline(&me, 10, None);
    for (i, t) in tl.iter().filter(|t| !t.message.attachments.is_empty()).enumerate() {
        let p = alice.download_attachment(&t.message.attachments[0]).await.unwrap();
        assert_eq!(std::fs::read(p).unwrap(), originals[i]);
    }
    let v = s.admin.verify_storage(true).await.unwrap();
    let v = s.admin.wait_job(&v.id).await.unwrap();
    assert_eq!((v.state.as_str(), v.done, v.failed), ("done", 3, 0));
    // Break one file in the new storage: verification reports it.
    let victim = std::fs::read_dir(target_dir.join("att")).unwrap().next().unwrap().unwrap().path();
    std::fs::remove_file(victim).unwrap();
    let v = s.admin.verify_storage(false).await.unwrap();
    let v = s.admin.wait_job(&v.id).await.unwrap();
    assert_eq!(v.state, "failed");
    assert_eq!(v.failed, 1);
}

#[tokio::test]
async fn privacy_flag_controls_last_active_visibility() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    // The flag is off by default: the user may not hide anything.
    let err = alice.update_profile(UpdateMeRequest { hide_last_active: Some(true), ..Default::default() }).await.unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::Forbidden));
    assert!(!alice.me().unwrap().privacy_controls_enabled);
    // An admin selects Alice.
    s.admin.update_user(&a_id, &UpdateUserRequest { privacy_controls_enabled: Some(true), ..Default::default() }).await.unwrap();
    wait_until("alice sees flag", || alice.me().map(|m| m.privacy_controls_enabled).unwrap_or(false)).await;
    let me = alice.update_profile(UpdateMeRequest { hide_last_active: Some(true), ..Default::default() }).await.unwrap();
    assert!(me.hide_last_active);
    // Bob no longer receives Alice's last-active time.
    let users: Vec<User> = bob.api().unwrap().get("/users").await.unwrap();
    let a = users.iter().find(|u| u.id == a_id).unwrap();
    assert!(a.last_active_ms.is_none());
    let b = users.iter().find(|u| u.id == b_id).unwrap();
    assert!(b.last_active_ms.is_some(), "users without the flag are unaffected");
    // Admin always sees the real value.
    let au = s.admin.users().await.unwrap().into_iter().find(|u| u.user.id == a_id).unwrap();
    assert!(au.last_active_real_ms.is_some());
}

#[tokio::test]
async fn presence_and_typing_events() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    wait_until("bob has dm", || bob.conversations().iter().any(|c| c.conversation.id == conv)).await;
    let mut rx = bob.subscribe();
    alice.typing(&conv);
    let mut got = false;
    for _ in 0..40 {
        if let Ok(Ok(client_core::ClientEvent::Typing { conv_id, user_id })) = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await {
            assert_eq!((conv_id, user_id), (conv.clone(), a_id.clone()));
            got = true;
            break;
        }
    }
    assert!(got, "typing indicator delivered");
    // Presence: set DND and see it on the other side.
    alice.set_presence_pref("dnd").await.unwrap();
    wait_until("bob sees dnd", || bob.user(&a_id).map(|u| u.presence == Some(Presence::Dnd)).unwrap_or(false)).await;
    alice.set_presence_pref("auto").await.unwrap();
    wait_until("bob sees online", || bob.user(&a_id).map(|u| u.presence == Some(Presence::Online)).unwrap_or(false)).await;
}

#[tokio::test]
async fn history_pagination_and_jump_to_date() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let me = alice.self_conversation().unwrap().conversation.id;
    for i in 0..130 {
        alice.send_text(&me, &format!("note {i}"), SendOptions::default()).unwrap();
    }
    wait_until("all sent", || alice.timeline(&me, 500, None).iter().filter(|t| !t.local).count() >= 130).await;
    // A fresh device only gets the recent window, then loads older pages on scroll.
    let d2 = s.new_client("alice-phone");
    d2.login(&s.url, "alice", "password-123", None).await.unwrap();
    d2.start();
    wait_until("phone bootstrapped", || !d2.timeline(&me, 500, None).is_empty()).await;
    let initial = d2.timeline(&me, 500, None).len();
    assert!(initial < 130, "bootstrap returns only recent messages (got {initial})");
    assert!(d2.has_more_before(&me));
    let mut total = initial;
    while d2.has_more_before(&me) {
        total += d2.load_older(&me, None).await.unwrap();
    }
    assert_eq!(total, 130);
    assert_eq!(d2.timeline(&me, 500, None)[0].message.body, "note 0");
    // Jump to a date in the middle of the history.
    let mid_ts = d2.timeline(&me, 500, None)[60].message.ts;
    let m = d2.jump_to_date(&me, mid_ts).await.unwrap().unwrap();
    assert!(m.ts >= mid_ts);
}

#[tokio::test]
async fn cursor_expiry_triggers_rebootstrap() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    alice.send_text(&conv, "one", SendOptions::default()).unwrap();
    wait_until("bob one", || texts(&bob, &conv).len() == 1).await;
    bob.stop();
    alice.send_text(&conv, "two", SendOptions::default()).unwrap();
    wait_until("alice sent two", || alice.timeline(&conv, 5, None).iter().filter(|t| !t.local).count() == 2).await;
    // The server prunes its event log past bob's cursor.
    s.st.db.call(|c| { c.execute("UPDATE events SET ts=0", [])?; Ok(()) }).await.unwrap();
    let floor = atomic_server::prune_events(&s.st).await.unwrap();
    assert!(floor > bob.store().cursor());
    bob.start();
    wait_until("bob resynced", || texts(&bob, &conv) == vec!["one", "two"]).await;
}

#[tokio::test]
async fn old_clients_are_told_to_update() {
    let s = TestServer::start().await;
    let http = reqwest::Client::new();
    let r = http.get(format!("{}/api/v1/me", s.url)).header("X-Client-Version", "0.1.0").header("Authorization", "Bearer x").send().await.unwrap();
    assert_eq!(r.status().as_u16(), 403);
    let b: ErrorBody = r.json().await.unwrap();
    assert_eq!(b.error.code, ErrorCode::ClientTooOld);
    let meta: ServerMeta = http.get(format!("{}/api/v1/meta", s.url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(meta.api_version, API_VERSION);
    assert_eq!(meta.min_client_version, MIN_CLIENT_VERSION);
    assert!(!meta.setup_required);
}

#[tokio::test]
async fn registration_does_not_exist_and_setup_is_one_time() {
    let s = TestServer::start().await;
    let http = reqwest::Client::new();
    for path in ["/api/v1/register", "/api/v1/signup", "/api/v1/users"] {
        let r = http.post(format!("{}{}", s.url, path)).json(&serde_json::json!({"username":"x","password":"y"})).send().await.unwrap();
        assert!(r.status().is_client_error(), "{path} must not allow sign-up ({})", r.status());
    }
    let again = client_core::admin::AdminClient::setup(&s.url, &SetupRequest { username: "evil".into(), display_name: "Evil".into(), password: "password-123".into(), email: None, storage: None }).await;
    assert_eq!(again.err().and_then(|e| e.code()), Some(ErrorCode::SetupDone));
}

#[tokio::test]
async fn sessions_list_and_revoke() {
    let s = TestServer::start().await;
    let (_a, phone) = s.user("alice").await;
    let laptop = s.new_client("alice-laptop");
    laptop.login(&s.url, "alice", "password-123", None).await.unwrap();
    laptop.start();
    wait_online(&laptop).await;
    let list = phone.sessions().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list.iter().filter(|s| s.current).count(), 1);
    assert!(list.iter().any(|s| s.device_name == "test-alice-laptop"));
    let waiter = {
        let l = laptop.clone();
        tokio::spawn(async move { wait_signed_out(&l).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    phone.revoke_other_sessions().await.unwrap();
    let (code, _) = waiter.await.unwrap();
    assert_eq!(code, Some(ErrorCode::SessionRevoked));
    assert!(phone.signed_in());
}

#[tokio::test]
async fn token_refresh_is_transparent() {
    let s = TestServer::start_with(|c| c.access_ttl_secs = 1).await;
    s.admin.create_user("carol", "Carol", Some("password-123")).await.unwrap();
    let c = s.new_client("carol");
    c.login(&s.url, "carol", "password-123", None).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
    let before = c.session().unwrap().access_token;
    let me = c.refresh_me().await.unwrap();
    assert_eq!(me.user.username, "carol");
    assert_ne!(c.session().unwrap().access_token, before, "a fresh access token was obtained");
}

#[tokio::test]
async fn admin_removes_file_message_shows_file_removed() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let conv = alice.open_dm(&b_id).await.unwrap();
    let f = s.dir.path().join("x.bin");
    std::fs::write(&f, b"abc").unwrap();
    alice.send_files(&conv, "f", &[f], SendOptions::default()).unwrap();
    wait_until("bob gets", || bob.timeline(&conv, 5, None).iter().any(|t| !t.message.attachments.is_empty())).await;
    let att = bob.timeline(&conv, 5, None)[0].message.attachments[0].clone();
    s.admin.remove_file(&att.id).await.unwrap();
    wait_until("removed flag", || bob.timeline(&conv, 5, None)[0].message.attachments[0].removed).await;
    assert!(bob.download_attachment(&bob.timeline(&conv, 5, None)[0].message.attachments[0]).await.is_err());
}

#[tokio::test]
async fn uploads_and_backups_pause_while_storage_is_moving() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let me = alice.self_conversation().unwrap().conversation.id;
    s.st.migrating.store(true, std::sync::atomic::Ordering::SeqCst);
    let f = s.dir.path().join("x.txt");
    std::fs::write(&f, b"hello").unwrap();
    let cid = alice.send_files(&me, "", &[f], SendOptions::default()).unwrap();
    wait_until("upload refused", || alice.timeline(&me, 10, None).iter().any(|t| t.state == SendState::Failed)).await;
    alice.cancel_send(&cid).unwrap();
    // Clear history needs a backup first, and backups are refused while files are moving.
    assert_eq!(s.admin.clear_history(&a_id).await.unwrap_err().code(), Some(ErrorCode::BackupFailed));
    s.st.migrating.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(s.admin.backup_user(&a_id).await.is_ok());
}

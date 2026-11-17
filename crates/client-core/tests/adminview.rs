//! Admin chat viewer API and deletion with mandatory backup: single / bulk / date-range deletes for
//! everyone or for one person, restore of deleted messages, and refusal when the backup fails.
mod common;
use client_core::SendOptions;
use common::*;
use proto::*;

async fn setup() -> (TestServer, String, String, client_core::Client, client_core::Client, String) {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "general", &[&a_id, &b_id]).await;
    wait_conv(&bob, &ch).await;
    wait_conv(&alice, &ch).await;
    (s, a_id, b_id, alice, bob, ch)
}

#[tokio::test]
async fn viewer_lists_pages_and_searches_any_conversation() {
    let (s, a_id, b_id, alice, bob, ch) = setup().await;
    let dm = alice.open_dm(&b_id).await.unwrap();
    wait_conv(&bob, &dm).await;
    for i in 0..30 {
        say(&alice, &ch, &format!("message number {i:02} about apples")).await;
    }
    say(&bob, &dm, "private pears").await;
    let f = s.dir.path().join("pic.png");
    std::fs::write(&f, [0x89, b'P', b'N', b'G', 9, 9]).unwrap();
    bob.send_files(&ch, "photo", &[f], SendOptions::default()).unwrap();
    wait_msg(&alice, &ch, "photo").await;

    // Conversations: everything shared, filtered by kind, or one person's view.
    let all = s.admin.viewer_conversations(None, None, None).await.unwrap();
    assert!(all.iter().any(|c| c.title == "general" && c.message_count == 31));
    let dms = s.admin.viewer_conversations(None, Some("dm"), None).await.unwrap();
    assert!(dms.iter().any(|c| c.conversation.id == dm), "{dms:?}");
    let alice_view = s.admin.viewer_conversations(Some(&a_id), None, None).await.unwrap();
    let dm_row = alice_view.iter().find(|c| c.conversation.id == dm).unwrap();
    assert_eq!(dm_row.title, "Bob", "a DM is titled with the other person");
    assert_eq!(dm_row.last_message.as_ref().unwrap().body, "private pears");
    assert!(alice_view.iter().any(|c| c.title.starts_with("Notes to self")));
    assert_eq!(s.admin.viewer_conversations(None, None, Some("gener")).await.unwrap().len(), 1);

    // Paging: newest page, older page, a date window.
    let page = s.admin.viewer_messages(&ch, "limit=10").await.unwrap();
    assert_eq!(page.messages.len(), 10);
    assert!(page.has_more_before && !page.has_more_after);
    assert_eq!(page.messages.last().unwrap().body, "photo");
    assert_eq!(page.messages.last().unwrap().attachments.len(), 1, "photos come with their attachment metadata");
    let first_seq = page.messages[0].seq;
    let older = s.admin.viewer_messages(&ch, &format!("limit=10&before={first_seq}")).await.unwrap();
    assert_eq!(older.messages.len(), 10);
    assert!(older.messages.last().unwrap().seq < first_seq);
    let ts = older.messages[0].ts;
    let window = s.admin.viewer_messages(&ch, &format!("from_ms={ts}&to_ms={}&limit=200", older.messages[3].ts)).await.unwrap();
    assert!(window.messages.len() >= 4 && window.messages.iter().all(|m| m.ts >= ts && m.ts <= older.messages[3].ts));
    let around = s.admin.viewer_messages(&ch, &format!("around_id={}&limit=6", older.messages[5].id)).await.unwrap();
    assert!(around.messages.iter().any(|m| m.id == older.messages[5].id) && around.has_more_after && around.has_more_before);

    // Search across everything, within a date range, as a given user.
    let hits = s.admin.viewer_search("q=apples&limit=100").await.unwrap();
    assert_eq!(hits.len(), 30);
    assert!(hits[0].snippet.contains("[[apples]]"));
    let hits = s.admin.viewer_search("q=pears").await.unwrap();
    assert_eq!(hits.len(), 1, "admins can search private DMs of others");
    let now = atomic_server::db::now_ms();
    assert!(s.admin.viewer_search(&format!("q=apples&to_ms={}", now - 10_000_000)).await.unwrap().is_empty());
    assert_eq!(s.admin.viewer_search(&format!("q=apples&from_ms={}&to_ms={}", now - 10_000_000, now + 10_000_000)).await.unwrap().len(), 30);
    assert_eq!(s.admin.viewer_search(&format!("user_id={b_id}&q=pears")).await.unwrap().len(), 1);
    assert_eq!(s.admin.viewer_search("q=a&has=file").await.unwrap().len(), 0);
    assert_eq!(s.admin.viewer_search("has=file&limit=5").await.unwrap().len(), 1);
    // Normal users cannot use any of it.
    assert!(alice.api().unwrap().get::<Vec<ViewerConversation>>("/admin/viewer/conversations").await.is_err());
}

#[tokio::test]
async fn bulk_delete_for_everyone_backs_up_first_and_can_be_restored() {
    let (s, _a, _b, alice, bob, ch) = setup().await;
    let f = s.dir.path().join("evidence.txt");
    std::fs::write(&f, b"evidence bytes").unwrap();
    alice.send_files(&ch, "with attachment", &[f], SendOptions::default()).unwrap();
    wait_msg(&bob, &ch, "with attachment").await;
    let m1 = say(&alice, &ch, "delete me").await;
    let m2 = say(&bob, &ch, "delete me too").await;
    let keep = say(&bob, &ch, "keep me").await;
    let with_file = bob.timeline(&ch, 50, None).into_iter().find(|t| t.message.body == "with attachment").unwrap().message;
    let att = with_file.attachments[0].clone();

    // A plain user cannot do it.
    assert!(alice.api().unwrap().post::<AdminDeleteMessagesResponse, _>("/admin/messages/delete", &AdminDeleteMessagesRequest { message_ids: vec![m1.id.clone()], conv_id: None, from_ms: None, to_ms: None, before_ms: None, scope: "everyone".into(), user_id: None }).await.is_err());
    let r = s
        .admin
        .delete_messages(&AdminDeleteMessagesRequest { message_ids: vec![m1.id.clone(), m2.id.clone(), with_file.id.clone()], conv_id: None, from_ms: None, to_ms: None, before_ms: None, scope: "everyone".into(), user_id: None })
        .await
        .unwrap();
    assert_eq!(r.messages_deleted, 3);
    assert_eq!(r.attachments_moved, 1);
    let b = s.admin.backups().await.unwrap().into_iter().find(|b| b.id == r.backup_id).unwrap();
    assert_eq!(b.kind, "messages");
    // Both users now see tombstones; the file cannot be downloaded any more; unrelated messages stay.
    wait_until("tombstones", || alice.timeline(&ch, 50, None).iter().filter(|t| t.message.deleted).count() == 3 && bob.timeline(&ch, 50, None).iter().filter(|t| t.message.deleted).count() == 3).await;
    assert!(alice.timeline(&ch, 50, None).iter().any(|t| t.message.id == keep.id && !t.message.deleted));
    assert!(bob.download_attachment(&att).await.is_err());
    assert!(alice.search_local("delete").is_empty() || alice.search_local("delete").iter().all(|m| m.deleted));
    let hits = s.admin.viewer_search("q=delete").await.unwrap();
    assert!(hits.is_empty(), "deleted messages are not searchable");

    // Restore the backup: texts and the attachment return.
    let rr = s.admin.restore(&RestoreRequest { backup_id: r.backup_id.clone(), user_ids: vec![], all: false, mode: None }).await.unwrap();
    assert_eq!(rr.messages_added, 3, "{rr:?}");
    assert_eq!(rr.attachments_moved_back, 1, "{rr:?}");
    wait_until("restored for both", || texts(&alice, &ch).contains(&"delete me".to_string()) && texts(&bob, &ch).contains(&"delete me too".to_string())).await;
    let again = bob.timeline(&ch, 50, None).into_iter().find(|t| t.message.id == with_file.id).unwrap().message;
    let p = bob.download_attachment(&again.attachments[0]).await.unwrap();
    assert_eq!(std::fs::read(p).unwrap(), b"evidence bytes");
}

#[tokio::test]
async fn delete_for_one_person_only_and_by_date_range() {
    let (s, a_id, _b, alice, bob, ch) = setup().await;
    let old1 = say(&alice, &ch, "ancient one").await;
    let old2 = say(&bob, &ch, "ancient two").await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let cut = atomic_server::db::now_ms();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    say(&alice, &ch, "recent one").await;
    wait_msg(&bob, &ch, "recent one").await;

    // Scope "user": only alice's copy of the selected message disappears.
    let r = s
        .admin
        .delete_messages(&AdminDeleteMessagesRequest { message_ids: vec![old2.id.clone()], conv_id: None, from_ms: None, to_ms: None, before_ms: None, scope: "user".into(), user_id: Some(a_id.clone()) })
        .await
        .unwrap();
    assert_eq!(r.messages_deleted, 1);
    wait_until("alice lost it", || !texts(&alice, &ch).contains(&"ancient two".to_string())).await;
    assert!(texts(&bob, &ch).contains(&"ancient two".to_string()));
    // Restoring the message backup brings it back to alice only.
    s.admin.restore(&RestoreRequest { backup_id: r.backup_id, user_ids: vec![a_id.clone()], all: false, mode: None }).await.unwrap();
    wait_until("alice has it again", || texts(&alice, &ch).contains(&"ancient two".to_string())).await;

    // "Delete before date" for everyone in a conversation: older than the cut, nothing newer.
    let r = s
        .admin
        .delete_messages(&AdminDeleteMessagesRequest { message_ids: vec![], conv_id: Some(ch.clone()), from_ms: None, to_ms: None, before_ms: Some(cut), scope: "everyone".into(), user_id: None })
        .await
        .unwrap();
    assert_eq!(r.messages_deleted, 2, "only the two ancient messages");
    wait_until("ancient deleted for bob", || bob.timeline(&ch, 50, None).iter().filter(|t| t.message.deleted).count() == 2).await;
    assert!(texts(&bob, &ch).contains(&"recent one".to_string()));
    let _ = old1;
    // Range with nothing in it, and an inverted range, are mistakes.
    let none = s.admin.delete_messages(&AdminDeleteMessagesRequest { message_ids: vec![], conv_id: Some(ch.clone()), from_ms: Some(1), to_ms: Some(2), before_ms: None, scope: "everyone".into(), user_id: None }).await;
    assert!(none.is_err());
    let inverted = s.admin.delete_messages(&AdminDeleteMessagesRequest { message_ids: vec![], conv_id: Some(ch.clone()), from_ms: Some(10), to_ms: Some(2), before_ms: None, scope: "everyone".into(), user_id: None }).await;
    assert_eq!(inverted.unwrap_err().code(), Some(ErrorCode::Validation));
}

#[tokio::test]
async fn delete_range_by_user_supports_before_date_and_other_copies() {
    let (s, a_id, _b, alice, bob, ch) = setup().await;
    say(&alice, &ch, "old a").await;
    say(&bob, &ch, "old b").await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let cut = atomic_server::db::now_ms();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    say(&alice, &ch, "new a").await;
    wait_msg(&bob, &ch, "new a").await;
    // Default: only alice's own copy, only before the date.
    let r = s.admin.delete_range(&a_id, &DateRangeDeleteRequest { from_ms: 0, to_ms: 0, before_ms: Some(cut), conversation_id: Some(ch.clone()), also_other_copies: false }).await.unwrap();
    assert_eq!(r.messages_hidden, 2, "{r:?}");
    assert!(s.admin.backups().await.unwrap().iter().any(|b| b.user_id.as_deref() == Some(a_id.as_str())), "backup first");
    wait_until("alice sees only new", || texts(&alice, &ch) == vec!["new a".to_string()]).await;
    assert_eq!(texts(&bob, &ch).len(), 3, "the other person's copy stays");
    // Opt in: also delete the other person's copy of what alice wrote.
    let r = s.admin.delete_range(&a_id, &DateRangeDeleteRequest { from_ms: 0, to_ms: i64::MAX / 2, before_ms: None, conversation_id: Some(ch.clone()), also_other_copies: true }).await.unwrap();
    assert!(r.messages_hidden >= 1);
    wait_until("bob lost alice's copy", || !texts(&bob, &ch).contains(&"new a".to_string())).await;
}

#[tokio::test]
async fn deleting_is_refused_when_the_backup_cannot_be_written() {
    let (s, _a, _b, alice, bob, ch) = setup().await;
    let m = say(&alice, &ch, "must survive").await;
    wait_msg(&bob, &ch, "must survive").await;
    let bad = s.dir.path().join("not-a-dir");
    std::fs::write(&bad, b"x").unwrap();
    let ro = s.dir.path().join("ro");
    std::fs::create_dir_all(&ro).unwrap();
    let b = atomic_server::storage::build_storage(&StorageConfig::Local { path: Some(ro.to_string_lossy().to_string()) }, s.dir.path()).unwrap();
    std::fs::remove_dir_all(&ro).unwrap();
    std::fs::write(&ro, b"x").unwrap();
    *s.st.storage.write().unwrap() = b;
    let err = s
        .admin
        .delete_messages(&AdminDeleteMessagesRequest { message_ids: vec![m.id.clone()], conv_id: None, from_ms: None, to_ms: None, before_ms: None, scope: "everyone".into(), user_id: None })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some(ErrorCode::BackupFailed));
    assert!(texts(&bob, &ch).contains(&"must survive".to_string()));
    assert!(!bob.timeline(&ch, 10, None).iter().any(|t| t.message.deleted));
    assert!(s.admin.backups().await.unwrap().is_empty(), "no half-written backup record");
}

#[tokio::test]
async fn admin_can_chat_like_a_normal_user_in_conversations_they_belong_to() {
    let (s, a_id, _b, alice, _bob, ch) = setup().await;
    // The admin joins the channel and uses the ordinary user API (send, react, edit own, thread).
    let admin_id = s.admin.users().await.unwrap().into_iter().find(|u| u.user.username == "admin").unwrap().user.id;
    s.admin.add_members(&ch, &[admin_id.clone()]).await.unwrap();
    let call = |m: reqwest::Method, p: String, b: Option<serde_json::Value>| {
        let s = &s;
        async move { s.admin.call::<serde_json::Value>(m, &p, b).await.unwrap() }
    };
    let root: Message = serde_json::from_value(
        call(reqwest::Method::POST, format!("/conversations/{ch}/messages"), Some(serde_json::json!({"client_id": "adm1", "body": "hello from the admin viewer"}))).await,
    )
    .unwrap();
    wait_msg(&alice, &ch, "hello from the admin viewer").await;
    call(reqwest::Method::PUT, format!("/messages/{}/reactions/%F0%9F%91%8D", root.id), None).await;
    let edited: Message = serde_json::from_value(call(reqwest::Method::PATCH, format!("/messages/{}", root.id), Some(serde_json::json!({"body": "edited by admin", "mentions": []}))).await).unwrap();
    assert_eq!(edited.version, 2);
    let reply: Message = serde_json::from_value(
        call(reqwest::Method::POST, format!("/conversations/{ch}/messages"), Some(serde_json::json!({"client_id": "adm2", "body": "in a thread", "thread_root": root.id}))).await,
    )
    .unwrap();
    assert_eq!(reply.thread_root.as_deref(), Some(root.id.as_str()));
    wait_until("alice sees edit and reply", || texts(&alice, &ch).contains(&"edited by admin".to_string()) && alice.thread_replies(&ch, &root.id).len() == 1).await;
    let _ = a_id;
}

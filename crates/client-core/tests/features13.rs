//! 1.3: upload progress / cancel / retry (F-01), other people's profile photos (P-03).
mod common;
use client_core::{ClientEvent, SendOptions};
use common::*;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn upload_reports_progress_and_can_be_cancelled_before_it_finishes() {
    let s = TestServer::start_with(|c| c.max_upload_bytes = 400 * 1024 * 1024).await;
    let (alice_id, alice) = s.user("alice").await;
    let (bob_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "files", &[&alice_id, &bob_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&bob, &ch).await;
    let big = s.dir.path().join("big.bin");
    std::fs::write(&big, vec![7u8; 160 * 1024 * 1024]).unwrap();
    let mut rx = alice.subscribe();
    let cid = alice.send_files(&ch, "", std::slice::from_ref(&big), SendOptions::default()).unwrap();
    // The first progress event arrives while the file is still going up.
    let first = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(ClientEvent::UploadProgress { client_id, sent, total }) = rx.recv().await {
                if client_id == cid {
                    return (sent, total);
                }
            }
        }
    })
    .await
    .expect("no progress event");
    assert_eq!(first.1, 160 * 1024 * 1024);
    assert!(first.0 <= first.1);
    let st = alice.upload_progress(&cid).expect("progress is queryable");
    assert_eq!(st.file_name, "big.bin");
    assert_eq!(st.total, first.1);
    // Cancel: the message disappears from the timeline and nothing reaches the channel.
    alice.cancel_send(&cid).unwrap();
    assert!(alice.upload_progress(&cid).is_none());
    assert!(alice.timeline(&ch, 50, None).iter().all(|t| t.message.client_id.as_deref() != Some(cid.as_str())));
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(alice.timeline(&ch, 50, None).iter().all(|t| t.message.attachments.is_empty()), "cancelled upload must not be delivered");
    assert!(bob.timeline(&ch, 50, None).iter().all(|t| t.message.attachments.is_empty()));
    assert!(alice.store().outbox_pending().unwrap().is_empty());
    // The queue still works afterwards.
    let small = s.dir.path().join("small.txt");
    std::fs::write(&small, b"hello").unwrap();
    alice.send_files(&ch, "after cancel", &[small], SendOptions::default()).unwrap();
    let m = wait_msg(&bob, &ch, "after cancel").await;
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0].name, "small.txt");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_upload_keeps_finished_files_and_retry_completes() {
    let s = TestServer::start().await;
    let (alice_id, alice) = s.user("alice").await;
    let (bob_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "files", &[&alice_id, &bob_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&bob, &ch).await;
    let a = s.dir.path().join("a.txt");
    let b = s.dir.path().join("b.txt");
    std::fs::write(&a, b"first").unwrap();
    std::fs::write(&b, b"second").unwrap();
    // The second file vanishes before the upload: the item fails (not sent), the first file stays uploaded.
    let cid = alice.send_files(&ch, "two files", &[a, b.clone()], SendOptions::default()).unwrap();
    std::fs::remove_file(&b).unwrap();
    wait_until("item failed or sent", || {
        alice.store().outbox_pending().map(|v| v.is_empty()).unwrap_or(false) || alice.timeline(&ch, 20, None).iter().any(|t| t.message.client_id.as_deref() == Some(cid.as_str()) && t.state == client_core::SendState::Failed)
    })
    .await;
    let failed = alice.timeline(&ch, 20, None).into_iter().find(|t| t.message.client_id.as_deref() == Some(cid.as_str())).unwrap();
    if failed.state == client_core::SendState::Failed {
        // Put the file back and retry: only the missing file is uploaded again, the message is delivered once.
        std::fs::write(&b, b"second").unwrap();
        alice.retry_send(&cid).unwrap();
        let m = wait_msg(&bob, &ch, "two files").await;
        assert_eq!(m.attachments.len(), 2);
        let n = bob.timeline(&ch, 50, None).iter().filter(|t| t.message.body == "two files").count();
        assert_eq!(n, 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn other_peoples_profile_photos_are_downloaded_and_cached() {
    let s = TestServer::start().await;
    let (alice_id, alice) = s.user("alice").await;
    let (_bob_id, bob) = s.user("bob").await;
    assert!(bob.avatar_path(&alice_id).is_none(), "no photo yet: initials");
    let photo = s.dir.path().join("me.png");
    std::fs::write(&photo, b"\x89PNG\r\n\x1a\nnot a real image but good enough for the cache").unwrap();
    alice.set_avatar(&photo).await.unwrap();
    assert!(alice.avatar_path(&alice_id).is_some(), "own photo is cached at once");
    wait_until("bob has alice's photo", || bob.avatar_path(&alice_id).is_some()).await;
    let p = bob.avatar_path(&alice_id).unwrap();
    assert!(std::fs::read(p).unwrap().starts_with(b"\x89PNG"));
    // Removing the photo goes back to initials.
    alice.update_profile(proto::UpdateMeRequest { avatar: Some(String::new()), ..Default::default() }).await.unwrap();
    wait_until("bob drops the photo", || bob.avatar_path(&alice_id).is_none()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_deletes_a_channel_with_messages_files_and_saved_items() {
    let s = TestServer::start().await;
    let (alice_id, alice) = s.user("alice").await;
    let (bob_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "doomed", &[&alice_id, &bob_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&bob, &ch).await;
    let f = s.dir.path().join("note.txt");
    std::fs::write(&f, b"remember").unwrap();
    alice.send_files(&ch, "with a file", &[f], SendOptions::default()).unwrap();
    let m = wait_msg(&bob, &ch, "with a file").await;
    bob.save_message(&m.id, None).await.unwrap();
    let _ = bob.pin_message(&ch, &m.id, true).await;
    // A member who is not an admin is refused; the owner/admin may delete.
    assert!(matches!(bob.delete_conversation(&ch).await, Err(client_core::ClientError::Api { code: proto::ErrorCode::Forbidden, .. })));
    s.admin.set_role(&alice_id, proto::Role::Admin).await.unwrap();
    wait_until("alice is admin", || alice.me().map(|m| m.user.role.is_admin()).unwrap_or(false)).await;
    alice.delete_conversation(&ch).await.unwrap();
    wait_until("bob lost the channel", || bob.conversations().iter().all(|c| c.conversation.id != ch)).await;
    wait_until("alice lost the channel", || alice.conversations().iter().all(|c| c.conversation.id != ch)).await;
    // Nothing is left on the server for it: no messages, no attachment rows, no saved items.
    let st = s.st.clone();
    let ch2 = ch.clone();
    let left: (i64, i64, i64) = st
        .db
        .call(move |c| {
            let msgs = c.query_row("SELECT COUNT(*) FROM messages WHERE conv_id=?1", [&ch2], |r| r.get(0))?;
            let saved = c.query_row("SELECT COUNT(*) FROM saved_items WHERE conv_id=?1", [&ch2], |r| r.get(0))?;
            let atts = c.query_row("SELECT COUNT(*) FROM attachments WHERE removed=0 AND purpose='file'", [], |r| r.get(0))?;
            Ok((msgs, saved, atts))
        })
        .await
        .unwrap();
    assert_eq!(left.0, 0);
    assert_eq!(left.1, 0);
    assert_eq!(left.2, 0, "the uploaded file was released");
}

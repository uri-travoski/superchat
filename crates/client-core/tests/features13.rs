//! 1.3: upload progress / cancel / retry (F-01), other people's profile photos (P-03).
mod common;
use client_core::{ClientEvent, SendOptions};
use common::*;
use std::time::Duration;

async fn wait_until_msg(c: &client_core::Client, conv: &str, f: impl Fn(&proto::Message) -> bool) -> proto::Message {
    for _ in 0..200 {
        if let Some(t) = c.timeline(conv, 200, None).into_iter().find(|t| f(&t.message)) {
            return t.message;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("message never arrived");
}

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

#[tokio::test(flavor = "multi_thread")]
async fn voice_message_carries_duration_and_waveform_to_the_recipient() {
    let s = TestServer::start().await;
    let (alice_id, alice) = s.user("alice").await;
    let (bob_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "voice", &[&alice_id, &bob_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&bob, &ch).await;
    // One second of a rising tone as a 16 kHz mono WAV.
    let rate = 16_000u32;
    let samples: Vec<i16> = (0..rate).map(|i| ((i as f32 / rate as f32) * 18000.0 * (i as f32 * 0.3).sin()) as i16).collect();
    let mut w = vec![];
    let dl = (samples.len() * 2) as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + dl).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), rate.to_le_bytes().to_vec(), (rate * 2).to_le_bytes().to_vec(), 2u16.to_le_bytes().to_vec(), 16u16.to_le_bytes().to_vec()] {
        w.extend_from_slice(&v);
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&dl.to_le_bytes());
    for x in samples {
        w.extend_from_slice(&x.to_le_bytes());
    }
    let p = s.dir.path().join("voice-message-1.wav");
    std::fs::write(&p, w).unwrap();
    alice.send_files(&ch, "", &[p], SendOptions::default()).unwrap();
    let m = wait_until_msg(&bob, &ch, |m| !m.attachments.is_empty()).await;
    let meta = m.attachments[0].meta.clone().expect("metadata reached the recipient");
    assert_eq!(meta.duration_ms, Some(1000));
    assert_eq!(meta.peaks.len(), 64);
    assert!(meta.peaks[60] > meta.peaks[4], "rising tone: {:?}", meta.peaks);
    // Only the uploader may set details.
    let att = m.attachments[0].id.clone();
    let r: Result<serde_json::Value, _> = bob.api().unwrap().put_json(&format!("/files/{att}/meta"), &proto::AttachmentMeta { duration_ms: Some(5), ..Default::default() }).await;
    assert!(r.is_err());
    // Oversized details are refused.
    let big = proto::AttachmentMeta { poster_b64: Some("x".repeat(60 * 1024)), ..Default::default() };
    let r: Result<serde_json::Value, _> = alice.api().unwrap().put_json(&format!("/files/{att}/meta"), &big).await;
    assert!(r.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_order_of_conversations_and_sections_syncs_between_devices() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, _bob) = s.user("bob").await;
    let c1 = make_channel(&s, "alpha", &[&a_id, &b_id]).await;
    let c2 = make_channel(&s, "beta", &[&a_id, &b_id]).await;
    let c3 = make_channel(&s, "gamma", &[&a_id, &b_id]).await;
    wait_conv(&alice, &c3).await;
    let alice2 = s.new_client("alice-phone");
    alice2.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    let all = [c1.clone(), c2.clone(), c3.clone()];
    let ids = |c: &client_core::Client| -> Vec<String> { c.sorted_conversations(true, false, false).iter().map(|x| x.conversation.id.clone()).filter(|i| all.contains(i)).collect() };
    assert_eq!(ids(&alice), vec![c1.clone(), c2.clone(), c3.clone()], "A-Z by default");

    // Conversations: a hand-made order beats the list setting, on both devices.
    alice.reorder_conversations(&[c3.clone(), c1.clone(), c2.clone()]).await.unwrap();
    assert_eq!(ids(&alice), vec![c3.clone(), c1.clone(), c2.clone()]);
    wait_until("order on device 2", || ids(&alice2) == vec![c3.clone(), c1.clone(), c2.clone()]).await;
    // Another user's order is not affected.
    assert_eq!(ids(&_bob), vec![c1.clone(), c2.clone(), c3.clone()]);
    // Unknown or foreign conversations are refused.
    assert!(alice.reorder_conversations(&["nope".to_string()]).await.is_err());
    // Reset: the list setting rules again.
    alice.reorder_conversations(&[]).await.unwrap();
    assert_eq!(ids(&alice), vec![c1.clone(), c2.clone(), c3.clone()]);
    wait_until("reset on device 2", || ids(&alice2) == vec![c1.clone(), c2.clone(), c3.clone()]).await;

    // Sections: default order, then a custom one, then a saved order that includes the built-in sections.
    assert_eq!(alice.section_order(), vec!["fav", "channels", "dms"]);
    let a = alice.create_section("Projects").await.unwrap()[0].id.clone();
    let b = alice.create_section("Later").await.unwrap().iter().find(|x| x.name == "Later").unwrap().id.clone();
    assert_eq!(alice.section_order(), vec!["fav".to_string(), a.clone(), b.clone(), "channels".into(), "dms".into()]);
    let want: Vec<String> = vec!["dms".into(), b.clone(), "fav".into(), "channels".into(), a.clone()];
    alice.reorder_sections(&want).await.unwrap();
    assert_eq!(alice.section_order(), want);
    wait_until("section order on device 2", || alice2.section_order() == want).await;
    // The order-only rows never show up as sections you can file conversations in.
    assert_eq!(alice.sections().len(), 2);
    assert!(alice.sections().iter().all(|x| x.builtin_key().is_none()));
    // A section created later lands before Channels.
    let c = alice.create_section("Fresh").await.unwrap().iter().find(|x| x.name == "Fresh").unwrap().id.clone();
    let order = alice.section_order();
    assert_eq!(order.iter().position(|k| *k == c).unwrap() + 1, order.iter().position(|k| k == "channels").unwrap());
    // Bad keys are refused; an empty list puts the built-in sections back (custom ones keep their relative order).
    assert!(alice.reorder_sections(&["builtin:nonsense".to_string()]).await.is_err());
    alice.reorder_sections(&[]).await.unwrap();
    assert_eq!(alice.section_order(), vec!["fav".to_string(), b, c, a, "channels".into(), "dms".into()]);
}

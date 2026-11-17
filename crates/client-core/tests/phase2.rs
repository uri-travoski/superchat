//! Phase 2: search filters, threads, mentions and groups, pins, bookmarks, preferences, drafts,
//! forwarding and permalinks, end to end through the real server and client-core.
mod common;
use client_core::logic::searchq;
use client_core::{ClientError, SendOptions};
use common::*;
use proto::*;
use std::time::Duration;

fn code(e: &ClientError) -> Option<ErrorCode> {
    e.code()
}

#[tokio::test]
async fn search_filters_and_strict_isolation() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let general = make_channel(&s, "general", &[&a_id, &b_id]).await;
    let secret = make_channel(&s, "secret", &[&a_id, &c_id]).await;
    wait_conv(&bob, &general).await;
    wait_conv(&carol, &secret).await;

    let m1 = say(&alice, &general, "budget review http://example.com/q4").await;
    let _m2 = say(&bob, &general, "budget numbers attached").await;
    let _m3 = say(&alice, &secret, "budget layoffs are secret").await;
    // A reaction and a file.
    bob.react(&m1.id, "👍", true).await.unwrap();
    let f = s.dir.path().join("report.txt");
    std::fs::write(&f, b"quarterly report").unwrap();
    alice.send_files(&general, "budget file", &[f.clone()], SendOptions::default()).unwrap();
    wait_until("file msg", || alice.timeline(&general, 50, None).iter().any(|t| t.message.body == "budget file" && !t.local)).await;

    let all = |f: &str| searchq::parse(f, 0);
    // Plain search for Bob: never the secret channel.
    let r = bob.search_filtered(&all("budget")).await.unwrap();
    assert_eq!(r.hits.len(), 3, "bob sees only general: {:?}", r.hits.iter().map(|h| &h.message.body).collect::<Vec<_>>());
    assert!(r.hits.iter().all(|h| h.message.conv_id == general));
    // Carol sees only the secret one.
    let r = carol.search_filtered(&all("budget")).await.unwrap();
    assert_eq!(r.hits.len(), 1);
    // Filters.
    assert_eq!(alice.search_filtered(&all("budget from:bob")).await.unwrap().hits.len(), 1);
    assert_eq!(alice.search_filtered(&all("budget in:#secret")).await.unwrap().hits.len(), 1);
    assert_eq!(alice.search_filtered(&all("budget has:link")).await.unwrap().hits.len(), 1);
    assert_eq!(alice.search_filtered(&all("budget has:file")).await.unwrap().hits.len(), 1);
    assert_eq!(alice.search_filtered(&all("budget has:reaction")).await.unwrap().hits.len(), 1);
    let future = searchq::parse("budget before:01/01/2100", 0);
    assert_eq!(alice.search_filtered(&future).await.unwrap().hits.len(), 4);
    let past = searchq::parse("budget before:01/01/2000", 0);
    assert_eq!(alice.search_filtered(&past).await.unwrap().hits.len(), 0);
    let after = searchq::parse("budget after:01/01/2000", 0);
    assert_eq!(alice.search_filtered(&after).await.unwrap().hits.len(), 4);
    let after_future = searchq::parse("budget after:01/01/2100", 0);
    assert_eq!(alice.search_filtered(&after_future).await.unwrap().hits.len(), 0);
    // Isolation when naming a conversation you are not in.
    let r: SearchResponse = bob.api().unwrap().get(&format!("/search?q=layoffs&in={secret}")).await.unwrap();
    assert!(r.hits.is_empty());
    let r: SearchResponse = bob.api().unwrap().get("/search?q=layoffs&from=alice").await.unwrap();
    assert!(r.hits.is_empty());

    // Files and people and own channels.
    let r = alice.search_filtered(&all("report")).await.unwrap();
    assert_eq!(r.files.len(), 1);
    assert_eq!(r.files[0].attachment.name, "report.txt");
    assert!(carol.search_filtered(&all("report")).await.unwrap().files.is_empty(), "files of other channels stay hidden");
    let r = alice.search_filtered(&all("car")).await.unwrap();
    assert!(r.people.iter().any(|u| u.username == "carol"));
    let r = bob.search_filtered(&all("sec")).await.unwrap();
    assert!(r.channels.is_empty(), "bob is not in #secret");
    let r = alice.search_filtered(&all("sec")).await.unwrap();
    assert_eq!(r.channels.len(), 1);
    // Files tab list with type filter.
    let files = alice.list_files(Some(&general), Some("document"), None, None).await.unwrap();
    assert_eq!(files.len(), 1);
    assert!(alice.list_files(Some(&general), Some("image"), None, None).await.unwrap().is_empty());
    assert!(bob.list_files(Some(&secret), None, None, None).await.unwrap().is_empty());
}

#[tokio::test]
async fn threads_replies_follow_and_unread() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let ch = make_channel(&s, "dev", &[&a_id, &b_id, &c_id]).await;
    wait_conv(&bob, &ch).await;
    wait_conv(&carol, &ch).await;

    let root = say(&alice, &ch, "who can review the PR?").await;
    wait_msg(&bob, &ch, "who can review the PR?").await;
    // Bob replies in the thread; the reply stays out of the main timeline.
    bob.send_thread_reply(&ch, &root.id, "I can").unwrap();
    wait_until("alice gets reply", || alice.thread_replies(&ch, &root.id).iter().any(|t| t.message.body == "I can")).await;
    assert!(alice.timeline(&ch, 50, None).iter().all(|t| t.message.body != "I can"), "thread replies are not in the channel timeline");
    assert_eq!(alice.reply_counts(&ch).get(&root.id), Some(&1));
    // Alice (root author) follows automatically and has an unread thread; Carol is not following.
    wait_until("alice thread", || alice.threads().iter().any(|t| t.root.id == root.id && t.unread == 1)).await;
    assert!(carol.threads().is_empty());
    // Bob (replier) follows automatically.
    wait_until("bob follows", || bob.threads().iter().any(|t| t.root.id == root.id && t.following)).await;

    // Carol follows manually, then gets the next reply.
    carol.follow_thread(&root.id, true).await.unwrap();
    alice.send_thread_reply(&ch, &root.id, "thanks!").unwrap();
    wait_until("carol unread thread", || carol.threads().iter().any(|t| t.root.id == root.id && t.reply_count == 2 && t.unread >= 1)).await;
    let v = carol.open_thread(&ch, &root.id).await.unwrap();
    assert_eq!(v.replies.len(), 2);
    assert_eq!(v.summary.reply_count, 2);
    // Reading clears it; unfollowing removes it from the list.
    carol.mark_thread_read(&root.id).await.unwrap();
    assert!(carol.threads().iter().all(|t| t.unread == 0));
    carol.follow_thread(&root.id, false).await.unwrap();
    assert!(carol.threads().is_empty());
    // Unfollowed threads are not re-followed by someone else's reply...
    bob.send_thread_reply(&ch, &root.id, "np").unwrap();
    wait_until("alice sees 3", || alice.thread_replies(&ch, &root.id).len() == 3).await;
    assert!(carol.threads().is_empty());
    // ...and the server list matches.
    let mine = carol.refresh_threads().await.unwrap();
    assert!(mine.is_empty());
    let mine = alice.refresh_threads().await.unwrap();
    assert_eq!(mine.len(), 1);
    // A reply to a reply resolves to the same root.
    let reply = alice.thread_replies(&ch, &root.id)[0].message.clone();
    let m: Message = alice
        .api()
        .unwrap()
        .post(&format!("/conversations/{ch}/messages"), &SendMessage { client_id: "x1".into(), body: "nested".into(), thread_root: Some(reply.id.clone()), reply_to: None, mentions: vec![], attachment_ids: vec![] })
        .await
        .unwrap();
    assert_eq!(m.thread_root.as_deref(), Some(root.id.as_str()));
}

#[tokio::test]
async fn mentions_channel_here_and_user_groups() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let ch = make_channel(&s, "ops", &[&a_id, &b_id, &c_id]).await;
    wait_conv(&bob, &ch).await;
    wait_conv(&carol, &ch).await;
    // @channel mentions everyone.
    say(&alice, &ch, "@channel standup now").await;
    wait_until("bob mention", || bob.conversations().iter().any(|c| c.conversation.id == ch && c.mentions == 1)).await;
    wait_until("carol mention", || carol.conversations().iter().any(|c| c.conversation.id == ch && c.mentions == 1)).await;
    let m = bob.timeline(&ch, 5, None)[0].message.clone();
    assert!(m.mentions.contains(&c_id) && m.mentions.contains(&b_id));
    assert!(!m.mentions.contains(&a_id), "sender is not mentioned");

    // A user group expands to its members; handle clashes are refused.
    let g = s.admin.create_user_group("oncall", "On call", &[b_id.clone()]).await.unwrap();
    assert_eq!(g.handle, "oncall");
    assert!(s.admin.create_user_group("alice", "x", &[]).await.is_err(), "clashes with a username");
    assert!(s.admin.create_user_group("channel", "x", &[]).await.is_err(), "clashes with @channel");
    wait_until("groups sync", || alice.groups().iter().any(|g| g.handle == "oncall")).await;
    say(&alice, &ch, "@oncall please look").await;
    wait_until("bob mention 2", || bob.conversations().iter().any(|c| c.conversation.id == ch && c.mentions == 2)).await;
    assert!(carol.conversations().iter().any(|c| c.conversation.id == ch && c.mentions == 1), "carol is not in the group");
    // @here only reaches people who are online right now.
    carol.stop();
    tokio::time::sleep(Duration::from_millis(300)).await;
    say(&alice, &ch, "@here quick question").await;
    wait_until("bob here", || bob.conversations().iter().any(|c| c.conversation.id == ch && c.mentions == 3)).await;
    assert!(carol.conversations().iter().any(|c| c.conversation.id == ch && c.mentions == 1));
}

#[tokio::test]
async fn pins_and_bookmarks_with_permissions() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "docs", &[&a_id, &b_id]).await;
    wait_conv(&bob, &ch).await;
    let m = say(&alice, &ch, "important").await;
    wait_msg(&bob, &ch, "important").await;
    // Plain members cannot pin in channels they do not own.
    let e = bob.pin_message(&ch, &m.id, true).await.unwrap_err();
    assert_eq!(code(&e), Some(ErrorCode::Forbidden));
    // Admin is a member of the channel? Not by default: use the owner's API through a member admin.
    s.admin.add_members(&ch, &[]).await.ok();
    let pins = pin_as_admin(&s, &ch, &m.id).await;
    assert_eq!(pins.len(), 1);
    wait_until("pin synced", || bob.pins(&ch).len() == 1 && alice.pins(&ch).len() == 1).await;
    assert_eq!(bob.pins(&ch)[0].message_id, m.id);

    // Bookmarks: link and validation.
    let bm = bob_bookmark_attempt(&bob, &ch).await;
    assert_eq!(bm, Some(ErrorCode::Forbidden));
    let v: Vec<Bookmark> = admin_post(&s, &format!("/conversations/{ch}/bookmarks"), &BookmarkRequest { title: "Wiki".into(), url: Some("https://example.com/wiki".into()), attachment_id: None, position: None }).await;
    assert_eq!(v.len(), 1);
    wait_until("bookmark synced", || bob.bookmarks(&ch).len() == 1).await;
    let err: Result<Vec<Bookmark>, _> = s_admin_try(&s, &format!("/conversations/{ch}/bookmarks"), &BookmarkRequest { title: "bad".into(), url: None, attachment_id: None, position: None }).await;
    assert!(err.is_err(), "a bookmark needs a link or a file");
    let v = alice.delete_bookmark(&ch, &bob.bookmarks(&ch)[0].id).await;
    assert!(v.is_err() || v.unwrap().is_empty());
    // Unpin.
    let _ = unpin_as_admin(&s, &ch, &m.id).await;
    wait_until("unpinned", || bob.pins(&ch).is_empty()).await;
}

async fn pin_as_admin(s: &TestServer, ch: &str, mid: &str) -> Vec<Pin> {
    s.admin.call(reqwest::Method::PUT, &format!("/conversations/{ch}/pins/{mid}"), None).await.unwrap()
}
async fn unpin_as_admin(s: &TestServer, ch: &str, mid: &str) -> Vec<Pin> {
    s.admin.call(reqwest::Method::DELETE, &format!("/conversations/{ch}/pins/{mid}"), None).await.unwrap()
}
async fn admin_post<T: serde::de::DeserializeOwned, B: serde::Serialize>(s: &TestServer, path: &str, b: &B) -> T {
    s.admin.call(reqwest::Method::POST, path, Some(serde_json::to_value(b).unwrap())).await.unwrap()
}
async fn s_admin_try<T: serde::de::DeserializeOwned, B: serde::Serialize>(s: &TestServer, path: &str, b: &B) -> client_core::Result<T> {
    s.admin.call(reqwest::Method::POST, path, Some(serde_json::to_value(b).unwrap())).await
}
async fn bob_bookmark_attempt(bob: &client_core::Client, ch: &str) -> Option<ErrorCode> {
    bob.add_bookmark(ch, &BookmarkRequest { title: "x".into(), url: Some("https://x.test".into()), attachment_id: None, position: None }).await.err().and_then(|e| e.code())
}

#[tokio::test]
async fn favourites_sections_mute_and_list_filters_sync_between_devices() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, _bob) = s.user("bob").await;
    let c1 = make_channel(&s, "alpha", &[&a_id, &b_id]).await;
    let c2 = make_channel(&s, "beta", &[&a_id, &b_id]).await;
    let c3 = make_channel(&s, "gamma", &[&a_id, &b_id]).await;
    wait_conv(&alice, &c3).await;
    alice.set_favourite(&c2, true).await.unwrap();
    alice.mute(&c3, "forever", 0).await.unwrap();
    assert!(alice.is_muted(&c3));
    let secs = alice.create_section("Projects").await.unwrap();
    alice.set_pref(&c1, &ConvPrefRequest { section_id: Some(secs[0].id.clone()), ..Default::default() }).await.unwrap();

    // A second device of the same account sees all of it after login (bootstrap) ...
    let alice2 = s.new_client("alice-phone");
    alice2.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    wait_until("prefs on device 2", || alice2.pref(&c2).favourite && alice2.is_muted(&c3) && alice2.pref(&c1).section_id.is_some()).await;
    assert_eq!(alice2.sections().len(), 1);
    // ... and live updates.
    alice.set_favourite(&c1, true).await.unwrap();
    wait_until("live fav", || alice2.pref(&c1).favourite).await;
    alice.mute(&c3, "off", 0).await.unwrap();
    wait_until("unmuted live", || !alice2.is_muted(&c3)).await;

    // Ordering and filters.
    let order: Vec<String> = alice.sorted_conversations(true, false, false).iter().map(|c| c.conversation.id.clone()).collect();
    let fav_count = 2;
    assert!(order[..fav_count].contains(&c1) && order[..fav_count].contains(&c2), "favourites first: {order:?}");
    alice.mute(&c3, "8h", 0).await.unwrap();
    assert!(alice.sorted_conversations(false, false, true).iter().all(|c| c.conversation.id != c3));
    assert!(alice.mute(&c3, "bogus", 0).await.is_err());
    // Deleting a section un-files its conversations.
    alice.delete_section(&secs[0].id).await.unwrap();
    wait_until("section gone", || alice2.sections().is_empty() && alice2.pref(&c1).section_id.is_none()).await;
}

#[tokio::test]
async fn drafts_sync_between_devices() {
    let s = TestServer::start().await;
    let (_a, alice) = s.user("alice").await;
    let (b_id, _bob) = s.user("bob").await;
    let dm = alice.open_dm(&b_id).await.unwrap();
    let alice2 = s.new_client("alice-2");
    alice2.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    wait_conv(&alice2, &dm).await;
    alice.save_draft(&dm, None, "half written thought");
    wait_until("draft on device 2", || alice2.draft(&dm) == "half written thought").await;
    alice.save_draft(&dm, Some("t1"), "thread draft");
    wait_until("thread draft on device 2", || alice2.thread_draft(&dm, "t1") == "thread draft").await;
    assert_eq!(alice2.all_drafts().len(), 2);
    // Clearing it (or sending) clears everywhere.
    alice.save_draft(&dm, None, "");
    wait_until("draft cleared", || alice2.draft(&dm).is_empty()).await;
}

#[tokio::test]
async fn forwarding_quote_reply_and_permalinks() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let src = make_channel(&s, "source", &[&a_id, &b_id]).await;
    let dst = make_channel(&s, "dest", &[&a_id, &c_id]).await;
    wait_conv(&alice, &dst).await;
    wait_conv(&carol, &dst).await;
    wait_conv(&bob, &src).await;
    let f = s.dir.path().join("pic.png");
    std::fs::write(&f, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
    alice.send_files(&src, "look at this", &[f], SendOptions::default()).unwrap();
    wait_until("sent", || alice.timeline(&src, 5, None).iter().any(|t| t.message.body == "look at this" && !t.local)).await;
    let orig = alice.timeline(&src, 5, None).into_iter().find(|t| t.message.body == "look at this").unwrap().message;
    // Carol is not in #source: she cannot forward or read it.
    assert!(carol.forward_message(&orig.id, &[dst.clone()], None).await.is_err());
    let fw = alice.forward_message(&orig.id, &[dst.clone()], Some("FYI")).await.unwrap();
    assert_eq!(fw.len(), 2, "forwarded copy plus the comment");
    let fwd = fw.iter().find(|m| m.forwarded.is_some()).unwrap();
    assert_eq!(fwd.forwarded.as_ref().unwrap().sender_id, a_id);
    assert_eq!(fwd.forwarded.as_ref().unwrap().message_id, orig.id);
    let got = wait_msg(&carol, &dst, "look at this").await;
    assert!(got.forwarded.is_some());
    assert_eq!(got.attachments.len(), 1, "forwarded attachment is re-used");
    // Carol can download it even though she is not in the source channel.
    let path = carol.download_attachment(&got.attachments[0]).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap().len(), 7);
    wait_until("comment sent", || texts(&carol, &dst).iter().any(|t| t == "FYI")).await;
    // Deleting the original does not delete the forwarded copy (F-04).
    alice.delete_message(&orig.id).await.unwrap();
    assert!(carol.download_attachment(&got.attachments[0]).await.is_ok());

    // Permalinks resolve for members only.
    let m = say(&alice, &dst, "see here").await;
    let link = alice.permalink(&m);
    assert!(link.starts_with("atomic-chat://m?"));
    let opened = carol.open_permalink(&link).await.unwrap();
    assert_eq!(opened.id, m.id);
    let err = bob.open_permalink(&link).await.unwrap_err();
    assert!(matches!(code(&err), Some(ErrorCode::Forbidden) | Some(ErrorCode::NotFound)), "{err:?}");
    assert!(bob.open_permalink("https://example.com").await.is_err());

    // Quote reply: reply_to points at the original, within the same conversation.
    carol.send_text(&dst, "quoting", SendOptions { thread_root: None, reply_to: Some(m.id.clone()) }).unwrap();
    let q = wait_msg(&alice, &dst, "quoting").await;
    assert_eq!(q.reply_to.as_deref(), Some(m.id.as_str()));
}

#[tokio::test]
async fn archive_is_read_only_and_reversible_and_admin_deletes_channels() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let ch = make_channel(&s, "old-project", &[&a_id]).await;
    wait_conv(&alice, &ch).await;
    say(&alice, &ch, "before archive").await;
    let path = format!("/conversations/{ch}");
    let call = |body: serde_json::Value| s.admin.call::<serde_json::Value>(reqwest::Method::PATCH, &path, Some(body));
    call(serde_json::json!({"archived": true})).await.unwrap();
    wait_until("archived syncs", || alice.conversations().iter().any(|c| c.conversation.id == ch && c.conversation.archived)).await;
    alice.send_text(&ch, "should fail", SendOptions::default()).unwrap();
    wait_until("send refused", || alice.timeline(&ch, 10, None).iter().any(|t| t.message.body == "should fail" && t.state == client_core::SendState::Failed)).await;
    call(serde_json::json!({"archived": false})).await.unwrap();
    wait_until("unarchived", || alice.conversations().iter().any(|c| c.conversation.id == ch && !c.conversation.archived)).await;
    // Admin deletes the channel: it disappears for members.
    let _: serde_json::Value = s.admin.call(reqwest::Method::DELETE, &format!("/admin/conversations/{ch}"), None).await.unwrap();
    wait_until("channel gone", || alice.conversations().iter().all(|c| c.conversation.id != ch)).await;
}

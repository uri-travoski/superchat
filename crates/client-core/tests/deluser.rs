//! Regression: deleting an account that has signed in (device sessions, push registrations, drafts, prefs ...) used to
//! answer 500; deleting must cascade and leave the other person's copy of the chats alone.
mod common;
use common::*;
use proto::*;

#[tokio::test]
async fn deleting_a_user_who_has_signed_in_works_and_keeps_the_other_copy() {
    let s = TestServer::start().await;
    let (b_id, bob) = s.user("bob").await;
    // Carol signs in through a one-time link, on two devices, with a push registration, a draft, a preference.
    let r = s.admin.create_user("carol", "Carol", None).await.unwrap();
    let c_id = r.user.user.id.clone();
    let carol = s.new_client("carol");
    carol.login_with_link(&r.login_link.unwrap().link, None, None).await.unwrap();
    carol.change_password(None, "carol-password-1").await.unwrap();
    carol.start();
    wait_online(&carol).await;
    let carol2 = s.new_client("carol2");
    carol2.login(&s.url, "carol", "carol-password-1", None).await.unwrap();
    carol.register_push(&PushRegistration { id: String::new(), kind: "ntfy".into(), target: "topic-carol".into(), ntfy_url: None, device_name: "phone".into(), created_ms: 0 }).await.unwrap();
    let dm = carol.open_dm(&b_id).await.unwrap();
    wait_conv(&bob, &dm).await;
    let ch = make_channel(&s, "team", &[&b_id, &c_id]).await;
    wait_conv(&carol, &ch).await;
    say(&carol, &dm, "hello from carol").await;
    say(&carol, &ch, "channel hello").await;
    wait_msg(&bob, &dm, "hello from carol").await;
    carol.save_draft(&ch, None, "half written");
    carol.set_favourite(&ch, true).await.unwrap();
    carol.mark_read(&dm).await.unwrap();
    let bot = s.admin.create_bot("tester_bot", "Tester", "t", &[ch.clone()]).await.unwrap();
    let _ = bot;

    s.admin.delete_user(&c_id).await.expect("deleting a signed-in user");
    wait_signed_out(&carol).await;
    assert!(s.new_client("carol3").login(&s.url, "carol", "carol-password-1", None).await.is_err());
    // Bob keeps his copy of the DM and the channel history.
    assert!(texts(&bob, &dm).contains(&"hello from carol".to_string()));
    assert!(texts(&bob, &ch).contains(&"channel hello".to_string()));
    // Deleting again is a clean error, not a 500.
    let e = s.admin.delete_user(&c_id).await;
    assert!(e.is_err() && e.unwrap_err().code() != Some(ErrorCode::Internal));
    // A second account can be deleted too (the placeholder user names must not collide).
    let (d_id, dave) = s.user("dave").await;
    let (e_id, _erin) = s.user("erin").await;
    s.admin.delete_user(&d_id).await.expect("second deletion");
    s.admin.delete_user(&e_id).await.expect("third deletion");
    wait_signed_out(&dave).await;
    // The server still works for everyone else.
    say(&bob, &dm, "still here").await;
}

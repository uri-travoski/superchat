//! 1.4.1: channel pictures everyone sees, and private pictures a person chose for other users, DMs, groups and channels.
mod common;
use client_core::admin::AdminClient;
use common::*;
use proto::*;
use std::path::PathBuf;

fn pic(s: &TestServer, name: &str, tag: u8) -> PathBuf {
    let p = s.dir.path().join(name);
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend(std::iter::repeat(tag).take(64));
    std::fs::write(&p, b).unwrap();
    p
}

fn bytes_of(p: Option<PathBuf>) -> Option<u8> {
    p.map(|p| *std::fs::read(p).unwrap().last().unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn admins_set_a_channel_picture_everyone_sees_and_members_cannot() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "design", &[&a_id, &b_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&bob, &ch).await;
    assert!(bob.conversation_avatar_path(&ch).is_none(), "initials until a picture is set");
    // A plain member may not set it.
    assert!(bob.set_conversation_avatar(&ch, &pic(&s, "b.png", 1)).await.is_err());
    // A server admin may (alice is made an admin).
    s.admin.set_role(&a_id, Role::Admin).await.unwrap();
    wait_until("alice sees she is admin", || alice.me().map(|m| m.user.role.is_admin()).unwrap_or(false)).await;
    alice.set_conversation_avatar(&ch, &pic(&s, "a.png", 7)).await.unwrap();
    assert_eq!(bytes_of(alice.conversation_avatar_path(&ch)), Some(7));
    wait_until("bob has the channel picture", || bytes_of(bob.conversation_avatar_path(&ch)) == Some(7)).await;
    // Removal falls back to initials for everyone.
    alice.clear_conversation_avatar(&ch).await.unwrap();
    wait_until("bob drops it", || bob.conversation_avatar_path(&ch).is_none()).await;
    assert!(alice.conversation_avatar_path(&ch).is_none());
    // Channels you are not in are not reachable.
    let (c_id, carol) = s.user("carol").await;
    let _ = c_id;
    assert!(carol.set_conversation_avatar(&ch, &pic(&s, "c.png", 3)).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn personal_pictures_are_private_win_over_the_real_one_and_fall_back_when_removed() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let ch = make_channel(&s, "general", &[&a_id, &b_id, &c_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&carol, &ch).await;
    // Bob has a profile picture of his own (tag 2).
    bob.set_avatar(&pic(&s, "bob.png", 2)).await.unwrap();
    wait_until("alice and carol see bob's photo", || bytes_of(alice.avatar_path(&b_id)) == Some(2) && bytes_of(carol.avatar_path(&b_id)) == Some(2)).await;

    // Alice sets a private picture for bob: she sees it, nobody else does.
    alice.set_avatar_override("user", &b_id, &pic(&s, "mine.png", 9)).await.unwrap();
    assert_eq!(bytes_of(alice.avatar_path(&b_id)), Some(9), "personal override wins");
    assert!(alice.has_avatar_override("user", &b_id));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(bytes_of(carol.avatar_path(&b_id)), Some(2), "carol still sees bob's own photo");
    assert_eq!(bytes_of(bob.avatar_path(&b_id)), Some(2), "bob sees his own photo");
    assert!(!carol.has_avatar_override("user", &b_id) && !bob.has_avatar_override("user", &b_id));
    // The file itself is readable only by the person who chose it (not by bob or carol, even with its id).
    let att_id = alice.avatar_override("user", &b_id).unwrap();
    let probe = Attachment { id: att_id.clone(), name: "x.png".into(), content_type: "image/png".into(), size: 1, removed: false, meta: None };
    assert!(bob.download_attachment(&probe).await.is_err() && carol.download_attachment(&probe).await.is_err());

    // A second device of alice gets it through sync.
    let alice2 = s.new_client("alice-2");
    alice2.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    wait_until("second device shows the override", || bytes_of(alice2.avatar_path(&b_id)) == Some(9)).await;

    // A private picture for a channel, with the channel's own picture set by an admin underneath.
    s.admin.set_role(&c_id, Role::Admin).await.unwrap();
    wait_until("carol is admin", || carol.me().map(|m| m.user.role.is_admin()).unwrap_or(false)).await;
    carol.set_conversation_avatar(&ch, &pic(&s, "chan.png", 4)).await.unwrap();
    wait_until("alice has the channel picture", || bytes_of(alice.conversation_avatar_path(&ch)) == Some(4)).await;
    alice.set_avatar_override("conv", &ch, &pic(&s, "mychan.png", 5)).await.unwrap();
    assert_eq!(bytes_of(alice.conversation_avatar_path(&ch)), Some(5));
    assert_eq!(bytes_of(bob.conversation_avatar_path(&ch)), Some(4), "bob sees the channel's picture, not alice's");
    wait_until("alice's second device too", || bytes_of(alice2.conversation_avatar_path(&ch)) == Some(5)).await;

    // Removal: the real picture (or initials) shows again, on every device of alice.
    alice.clear_avatar_override("user", &b_id).await.unwrap();
    assert_eq!(bytes_of(alice.avatar_path(&b_id)), Some(2));
    wait_until("second device follows the removal", || bytes_of(alice2.avatar_path(&b_id)) == Some(2)).await;
    alice.clear_avatar_override("conv", &ch).await.unwrap();
    assert_eq!(bytes_of(alice.conversation_avatar_path(&ch)), Some(4));
    // Bob removes his own photo: with no override, alice sees initials; with one she keeps hers.
    alice.set_avatar_override("user", &b_id, &pic(&s, "mine2.png", 8)).await.unwrap();
    bob.update_profile(UpdateMeRequest { avatar: Some(String::new()), ..Default::default() }).await.unwrap();
    wait_until("carol drops bob's photo", || carol.avatar_path(&b_id).is_none()).await;
    assert_eq!(bytes_of(alice.avatar_path(&b_id)), Some(8), "the override still shows");
    alice.clear_avatar_override("user", &b_id).await.unwrap();
    assert!(alice.avatar_path(&b_id).is_none(), "initials");
    // Not for yourself, not for strangers' conversations.
    assert!(alice.set_avatar_override("user", &a_id, &pic(&s, "me.png", 1)).await.is_err());
    let other = make_channel(&s, "secret", &[&b_id]).await;
    assert!(alice.set_avatar_override("conv", &other, &pic(&s, "no.png", 1)).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn personal_pictures_are_in_the_per_user_backup_and_come_back_on_restore() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, _bob) = s.user("bob").await;
    alice.set_avatar_override("user", &b_id, &pic(&s, "one.png", 6)).await.unwrap();
    let info = s.admin.backup_user(&a_id).await.unwrap();
    // Changed afterwards: replace-restore makes the view match the backup again.
    alice.set_avatar_override("user", &b_id, &pic(&s, "two.png", 7)).await.unwrap();
    assert_eq!(bytes_of(alice.avatar_path(&b_id)), Some(7));
    s.admin.restore(&RestoreRequest { backup_id: info.id.clone(), user_ids: vec![a_id.clone()], all: false, mode: Some("replace".into()) }).await.unwrap();
    wait_until("the backed-up picture is back", || bytes_of(alice.avatar_path(&b_id)) == Some(6)).await;
    // Removed afterwards: a merge restore brings it back.
    alice.clear_avatar_override("user", &b_id).await.unwrap();
    s.admin.restore(&RestoreRequest { backup_id: info.id, user_ids: vec![a_id.clone()], all: false, mode: None }).await.unwrap();
    wait_until("restored by merge", || bytes_of(alice.avatar_path(&b_id)) == Some(6)).await;
    let _: &AdminClient = &s.admin;
}

//! 1.2: membership management by owners/admins, Save for later + labels, Activity feed.
mod common;
use common::*;
use proto::*;

fn code_of<T: std::fmt::Debug>(r: client_core::error::Result<T>) -> ErrorCode {
    match r.unwrap_err() {
        client_core::ClientError::Api { code, .. } => code,
        other => panic!("unexpected error {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn admins_add_and_remove_members_and_members_cannot() {
    let s = TestServer::start().await;
    let (alice_id, alice) = s.user("alice").await;
    let (bob_id, bob) = s.user("bob").await;
    let (carol_id, carol) = s.user("carol").await;
    let (dave_id, dave) = s.user("dave").await;
    let ch = make_channel(&s, "ops", &[&alice_id, &bob_id]).await;
    s.admin.set_role(&bob_id, Role::Admin).await.unwrap();
    wait_until("bob sees admin", || bob.me().map(|m| m.user.role.is_admin()).unwrap_or(false)).await;
    wait_conv(&bob, &ch).await;
    wait_conv(&alice, &ch).await;
    assert!(bob.can_manage_members(&ch));
    assert!(!alice.can_manage_members(&ch));
    // A plain member is refused; an outsider cannot even tell the channel exists.
    assert_eq!(code_of(alice.add_members(&ch, std::slice::from_ref(&carol_id)).await), ErrorCode::Forbidden);
    assert_eq!(code_of(carol.add_members(&ch, std::slice::from_ref(&carol_id)).await), ErrorCode::NotFound);
    assert_eq!(code_of(alice.remove_member(&ch, &bob_id).await), ErrorCode::Forbidden);
    // An admin who is a member adds two people at once.
    bob.add_members(&ch, &[carol_id.clone(), dave_id.clone()]).await.unwrap();
    wait_conv(&carol, &ch).await;
    wait_conv(&dave, &ch).await;
    wait_until("alice sees 4 members", || alice.conversations().iter().find(|c| c.conversation.id == ch).map(|c| c.conversation.members.len() == 4).unwrap_or(false)).await;
    // The channel shows a system message and it is not unread noise.
    let sys = wait_until_msg(&alice, &ch, |m| m.kind == MessageKind::System).await;
    assert_eq!(sys.body, "Bob added Carol and Dave");
    assert_eq!(alice.conversations().iter().find(|c| c.conversation.id == ch).unwrap().unread, 0);
    // Adding the same person again changes nothing (no second message).
    let before = alice.timeline(&ch, 100, None).len();
    bob.add_members(&ch, std::slice::from_ref(&carol_id)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(alice.timeline(&ch, 100, None).len(), before);
    // Remove one: they lose the channel, the others see who removed them.
    bob.remove_member(&ch, &dave_id).await.unwrap();
    wait_until("dave lost the channel", || !dave.conversations().iter().any(|c| c.conversation.id == ch)).await;
    wait_until_msg(&alice, &ch, |m| m.kind == MessageKind::System && m.body == "Bob removed Dave").await;
    // DMs and notes cannot be managed.
    let dm = bob.open_dm(&alice_id).await.unwrap();
    assert_eq!(code_of(bob.add_members(&dm, std::slice::from_ref(&carol_id)).await), ErrorCode::Validation);
}

async fn wait_until_msg(c: &client_core::Client, conv: &str, f: impl Fn(&Message) -> bool) -> Message {
    for _ in 0..200 {
        if let Some(t) = c.timeline(conv, 200, None).into_iter().find(|t| f(&t.message)) {
            return t.message;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("message never arrived");
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_items_and_labels_are_private_synced_and_survive_deletion() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let ch = make_channel(&s, "design", &[&a_id, &b_id]).await;
    wait_conv(&alice, &ch).await;
    wait_conv(&bob, &ch).await;
    let m1 = say(&alice, &ch, "first").await;
    let m2 = say(&alice, &ch, "second").await;
    wait_msg(&bob, &ch, "second").await;
    // Alice saves two messages and labels one with two labels.
    let urgent = alice.create_label("Urgent", "#d93025").await.unwrap();
    assert_eq!(urgent.len(), 1);
    let todo = alice.create_label("todo", "#0467dd").await.unwrap();
    let (l_urgent, l_todo) = (urgent[0].id.clone(), todo[1].id.clone());
    alice.save_message(&m1.id, None).await.unwrap();
    let it = alice.save_message(&m2.id, Some(&[l_urgent.clone(), l_todo.clone()])).await.unwrap();
    assert_eq!(it.label_ids.len(), 2);
    assert!(alice.is_saved(&m1.id));
    // A second device of Alice sees it through bootstrap; Bob never sees anything.
    let alice2 = s.new_client("alice2");
    alice2.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    wait_until("second device has saved items", || alice2.saved_items().len() == 2 && alice2.labels().len() == 2).await;
    assert!(bob.saved_items().is_empty() && bob.labels().is_empty());
    // Bob cannot label or unsave Alice's item; his own saves are his own.
    assert_eq!(code_of(bob.set_saved_labels(&m2.id, &[l_urgent.clone()]).await), ErrorCode::NotFound);
    bob.save_message(&m1.id, None).await.unwrap();
    assert_eq!(bob.saved_items().len(), 1);
    assert!(bob.labels().is_empty());
    assert_eq!(code_of(bob.save_message(&m1.id, Some(&[l_urgent.clone()])).await), ErrorCode::NotFound, "someone else's label id is unknown to Bob");
    // Rename, recolour, remove a label from an item through a label delete.
    alice.update_label(&l_todo, Some("To do"), Some("#7b61ff")).await.unwrap();
    assert_eq!(code_of(alice.create_label("urgent", "#000000").await), ErrorCode::Conflict, "names are unique per user, ignoring case");
    assert_eq!(code_of(alice.create_label("x", "red").await), ErrorCode::Validation);
    alice.set_saved_labels(&m1.id, std::slice::from_ref(&l_todo)).await.unwrap();
    alice.delete_label(&l_todo).await.unwrap();
    wait_until("label removed from items on both devices", || alice2.saved_items().iter().all(|v| !v.item.label_ids.contains(&l_todo)) && alice2.labels().len() == 1).await;
    // Deleting the message leaves a tombstone: the saved item shows "Message deleted".
    alice.delete_message(&m2.id).await.unwrap();
    wait_until("tombstone in saved", || alice.saved_items().iter().any(|v| v.item.message_id == m2.id && v.message.as_ref().map(|m| m.deleted).unwrap_or(false))).await;
    // Unsave.
    alice.unsave_message(&m1.id).await.unwrap();
    wait_until("unsaved everywhere", || alice2.saved_items().len() == 1).await;
    // Losing access to the conversation hides the item (and it does not leak into the list).
    bob.save_message(&m1.id, None).await.ok();
    s.admin.remove_member(&ch, &b_id).await.unwrap();
    wait_until("bob cannot see it any more", || bob.saved_items().is_empty() || bob.conversations().iter().all(|c| c.conversation.id != ch)).await;
    let fresh = bob.refresh_saved().await.unwrap();
    assert!(fresh.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_items_and_labels_are_in_the_per_user_backup_and_come_back() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let ch = make_channel(&s, "notes", &[&a_id]).await;
    wait_conv(&alice, &ch).await;
    let m = say(&alice, &ch, "remember this").await;
    let l = alice.create_label("Keep", "#0dbd8b").await.unwrap();
    alice.save_message(&m.id, Some(&[l[0].id.clone()])).await.unwrap();
    let bk = s.admin.backup_user(&a_id).await.unwrap();
    // Replace mode wipes the live state first; the backup restores it.
    alice.unsave_message(&m.id).await.unwrap();
    alice.delete_label(&l[0].id).await.unwrap();
    s.admin.restore(&RestoreRequest { backup_id: bk.id.clone(), user_ids: vec![a_id.clone()], all: false, mode: Some("replace".into()) }).await.unwrap();
    wait_until("saved item and label restored", || alice.saved_items().len() == 1 && alice.labels().len() == 1).await;
    let v = alice.saved_items().remove(0);
    assert_eq!(v.item.label_ids, vec![alice.labels()[0].id.clone()]);
    // A deleted account restores with its saved items.
    s.admin.delete_user(&a_id).await.unwrap();
    let bk2 = s.admin.backups().await.unwrap().into_iter().filter(|b| b.user_id.as_deref() == Some(a_id.as_str())).max_by_key(|b| b.created_ms).expect("deletion backup");
    s.admin.restore(&RestoreRequest { backup_id: bk2.id, user_ids: vec![a_id.clone()], all: false, mode: None }).await.unwrap();
    let back = s.new_client("alice-back");
    s.admin.reset_password(&a_id, Some("password-123")).await.ok();
    back.login(&s.url, "alice", "password-123", None).await.unwrap();
    back.start();
    wait_online(&back).await;
    wait_until("saved item after account restore", || back.saved_items().len() == 1 && back.labels().len() == 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn activity_feed_lists_mentions_reactions_and_thread_replies_with_read_state() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let ch = make_channel(&s, "team", &[&a_id, &b_id, &c_id]).await;
    for c in [&alice, &bob, &carol] {
        wait_conv(c, &ch).await;
    }
    let mine = say(&alice, &ch, "my announcement").await;
    // Bob mentions Alice, reacts to her message and replies in a thread under it; Carol is not involved.
    bob.send_text(&ch, "hello @alice please look", client_core::SendOptions::default()).unwrap();
    wait_msg(&alice, &ch, "hello @alice please look").await;
    bob.react(&mine.id, "\u{1F44D}", true).await.unwrap();
    bob.send_thread_reply(&ch, &mine.id, "a reply under your post").unwrap();

    wait_until("three activity items", || alice.activity().len() == 3).await;
    let kinds: Vec<String> = alice.activity().iter().map(|i| i.kind.clone()).collect();
    for k in ["mention", "reaction", "thread_reply"] {
        assert!(kinds.contains(&k.to_string()), "{k} missing in {kinds:?}");
    }
    assert_eq!(alice.activity_unread(), 3);
    assert!(carol.activity().is_empty());
    assert!(bob.activity().is_empty(), "your own actions are never activity");
    // Mark all read: the marker syncs, new items after it are unread again.
    alice.mark_activity_read().await.unwrap();
    assert_eq!(alice.activity_unread(), 0);
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    bob.react(&mine.id, "\u{2764}\u{FE0F}", true).await.unwrap();
    wait_until("new unread reaction", || alice.activity_unread() == 1).await;
    // Locked conversations are filtered at the client-core level.
    alice.applock().set_pin("123456", "123456", client_core::applock::wall_ms()).unwrap();
    alice.applock().set_conv_locked(&ch, true).unwrap();
    alice.applock().lock();
    assert!(alice.activity().is_empty() && alice.saved_items().is_empty());
}

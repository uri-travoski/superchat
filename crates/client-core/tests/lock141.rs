//! 1.4.1: App Lock transitions are reactive (every read follows the lock state at once, a LockChanged event says so),
//! and sharing into a locked conversation sends without unlocking or revealing it.
mod common;
use client_core::applock::{wall_ms, Unlock};
use client_core::ClientEvent;
use common::*;

/// Waits for the next LockChanged, skipping unrelated events.
async fn lock_event(rx: &mut tokio::sync::broadcast::Receiver<ClientEvent>) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match rx.recv().await {
                Ok(ClientEvent::LockChanged) => return true,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

fn has(c: &client_core::Client, id: &str) -> bool {
    c.conversations().iter().any(|x| x.conversation.id == id)
}

#[tokio::test(flavor = "multi_thread")]
async fn lock_unlock_and_relock_take_effect_everywhere_at_once() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let secret = make_channel(&s, "secret", &[&a_id, &b_id]).await;
    let open = make_channel(&s, "open", &[&a_id, &b_id]).await;
    wait_conv(&alice, &secret).await;
    wait_conv(&alice, &open).await;
    say(&bob, &secret, "zebra secret").await;
    say(&bob, &open, "zebra open").await;
    wait_msg(&alice, &secret, "zebra secret").await;
    wait_msg(&alice, &open, "zebra open").await;
    alice.applock().set_pin("482915", "482915", wall_ms()).unwrap();
    let mut rx = alice.subscribe();
    let unread0 = alice.unread_total();

    // 1. "Lock chat" while unlocked: gone from every read in the same instant, no refresh, no restart.
    alice.set_conversation_locked(&secret, true).unwrap();
    assert!(!has(&alice, &secret) && has(&alice, &open));
    assert!(alice.sorted_conversations(false, false, false).iter().all(|c| c.conversation.id != secret));
    assert_eq!(alice.search_local("zebra").len(), 1);
    assert!(alice.timeline(&secret, 20, None).is_empty());
    assert!(alice.unread_total() < unread0, "badge counts leave it out");
    assert!(!alice.applock().is_engaged(), "the app itself stays unlocked");
    assert!(lock_event(&mut rx).await, "views are told to read again");

    // 2. Lock the app, then unlock with the PIN: the chat is back in place.
    alice.lock_app();
    assert!(lock_event(&mut rx).await);
    assert!(!has(&alice, &secret));
    assert_eq!(alice.unlock_app("000000", wall_ms()), Unlock::Wrong);
    assert!(!has(&alice, &secret), "a wrong PIN reveals nothing");
    assert_eq!(alice.unlock_app("482915", wall_ms()), Unlock::Ok);
    assert!(lock_event(&mut rx).await);
    assert!(has(&alice, &secret) && alice.applock().is_visible_locked(&secret));
    assert_eq!(alice.search_local("zebra").len(), 2);
    assert_eq!(alice.unread_total(), unread0);

    // 3. Re-lock by the idle timer and by a long time in the background.
    assert!(!alice.lock_timers(None, wall_ms()));
    assert!(alice.lock_timers(None, wall_ms() + 6 * 60 * 1000), "idle five minutes");
    assert!(!has(&alice, &secret));
    assert_eq!(alice.unlock_app("482915", wall_ms()), Unlock::Ok);
    assert!(!alice.lock_timers(Some(true), wall_ms()));
    assert!(alice.lock_timers(Some(false), wall_ms() + 6 * 60 * 1000), "in the background for more than five minutes");
    assert!(!has(&alice, &secret));
    assert_eq!(alice.unlock_app("482915", wall_ms()), Unlock::Ok);

    // 4. Undo: unmarking brings it back immediately and forgets the mark.
    alice.set_conversation_locked(&secret, true).unwrap();
    assert!(!has(&alice, &secret));
    alice.set_conversation_locked(&secret, false).unwrap();
    assert!(has(&alice, &secret) && !alice.is_conv_locked(&secret));
}

#[tokio::test(flavor = "multi_thread")]
async fn sharing_to_a_locked_conversation_sends_without_unlocking_or_revealing_it() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let secret = make_channel(&s, "secret", &[&a_id, &b_id]).await;
    wait_conv(&alice, &secret).await;
    wait_conv(&bob, &secret).await;
    alice.applock().set_pin("482915", "482915", wall_ms()).unwrap();
    alice.set_conversation_locked(&secret, true).unwrap();
    alice.lock_app();
    let gen = alice.applock().generation();
    // The picker lists everything, locked ones included, while the lists themselves hide it.
    assert!(!has(&alice, &secret));
    assert!(alice.share_targets().iter().any(|c| c.conversation.id == secret), "share picker lists locked conversations");
    let f = s.dir.path().join("note.txt");
    std::fs::write(&f, b"shared file").unwrap();
    alice.share_to(&secret, "hello from another app", &[f]).unwrap();
    let m = wait_msg(&bob, &secret, "hello from another app").await;
    assert_eq!(m.attachments.len(), 1);
    alice.share_to(&secret, "plain text share", &[]).unwrap();
    wait_msg(&bob, &secret, "plain text share").await;
    // Nothing changed on alice's side: still locked, still hidden, no state change to react to.
    assert!(alice.applock().is_engaged() && alice.is_hidden(&secret) && !has(&alice, &secret));
    assert_eq!(alice.applock().generation(), gen, "sharing never touches the lock");
    assert!(alice.timeline(&secret, 20, None).is_empty(), "and no content is readable");
    // Opening it still needs the PIN.
    assert_eq!(alice.unlock_app("000000", wall_ms()), Unlock::Wrong);
    assert!(alice.timeline(&secret, 20, None).is_empty());
    assert_eq!(alice.unlock_app("482915", wall_ms()), Unlock::Ok);
    wait_until("sent messages are in the chat", || texts(&alice, &secret).contains(&"plain text share".to_string())).await;
}

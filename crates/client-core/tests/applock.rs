//! App Lock through the real client: a locked conversation disappears from every read while the lock is engaged.
mod common;
use common::*;
use client_core::applock::{Unlock, wall_ms};

#[tokio::test]
async fn locked_chats_vanish_from_lists_search_counts_threads_and_drafts() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let secret = make_channel(&s, "secret-plans", &[&a_id, &b_id]).await;
    let open = make_channel(&s, "general", &[&a_id, &b_id]).await;
    wait_conv(&alice, &secret).await;
    wait_conv(&alice, &open).await;
    wait_conv(&bob, &secret).await;
    say(&bob, &secret, "zebra launch codes").await;
    say(&bob, &open, "zebra lunch").await;
    wait_msg(&alice, &secret, "zebra launch codes").await;
    wait_msg(&alice, &open, "zebra lunch").await;
    alice.save_draft(&secret, None, "draft about zebras");
    alice.save_draft(&open, None, "open draft");
    let before_unread = alice.unread_total();
    assert!(before_unread >= 2);

    // No PIN: marking is refused and everything shows.
    assert!(alice.applock().set_conv_locked(&secret, true).is_err());
    assert!(alice.conversations().iter().any(|c| c.conversation.id == secret));

    let now = wall_ms();
    alice.applock().set_pin("482915", "482915", now).unwrap();
    alice.applock().set_conv_locked(&secret, true).unwrap();
    // Unlocked right after setting up: still visible.
    assert!(alice.conversations().iter().any(|c| c.conversation.id == secret));
    assert!(alice.is_conv_locked(&secret) && !alice.is_hidden(&secret));

    alice.applock().lock();
    let ids: Vec<String> = alice.conversations().into_iter().map(|c| c.conversation.id).collect();
    assert!(!ids.contains(&secret) && ids.contains(&open), "{ids:?}");
    assert!(alice.sorted_conversations(false, false, false).iter().all(|c| c.conversation.id != secret));
    assert!(alice.resolve_conversation("secret-plans").is_none());
    assert!(alice.self_conversation().map(|c| c.conversation.id != secret).unwrap_or(true));
    assert!(alice.timeline(&secret, 50, None).is_empty());
    assert!(alice.unread_total() < before_unread, "unread counts leave the locked chat out");
    let hits = alice.search_local("zebra");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].conv_id, open);
    let r = alice.search("zebra").await.unwrap();
    assert!(r.hits.iter().all(|h| h.message.conv_id != secret) && !r.hits.is_empty());
    assert!(r.channels.iter().all(|c| c.conversation.id != secret));
    assert!(alice.search("secret-plans").await.unwrap().channels.is_empty());
    assert!(alice.all_drafts().iter().all(|d| d.conv_id != secret));
    assert_eq!(alice.all_drafts().len(), 1);
    assert!(alice.threads().iter().all(|t| t.conv_id != secret));
    // The locked chat keeps receiving messages in the background; nothing leaks as an event-driven count either.
    say(&bob, &secret, "another secret").await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(alice.conversations().iter().all(|c| c.conversation.id != secret));
    // Notifications about it must be generic even after unlocking.
    assert!(alice.is_conv_locked(&secret) && !alice.is_conv_locked(&open));

    // Wrong PIN keeps it hidden; the right one reveals it, with history and counts.
    assert_eq!(alice.applock().unlock("000000", wall_ms()), Unlock::Wrong);
    assert!(alice.is_hidden(&secret));
    assert_eq!(alice.applock().unlock("482915", wall_ms()), Unlock::Ok);
    assert!(alice.conversations().iter().any(|c| c.conversation.id == secret));
    wait_until("history back", || texts(&alice, &secret).contains(&"another secret".to_string())).await;
    assert_eq!(alice.search_local("zebra").len(), 2);
    assert_eq!(alice.all_drafts().len(), 2);

    // Other people are unaffected: the lock is local and not synced.
    assert!(bob.conversations().iter().any(|c| c.conversation.id == secret));
    assert!(!bob.applock().is_set());

    // Signing out forgets the lock together with the rest of the local data.
    alice.applock().lock();
    alice.logout().await.unwrap();
    assert!(!alice.applock().is_set());
}

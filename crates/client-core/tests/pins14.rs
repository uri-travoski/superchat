//! 1.4: pinned conversations on the phone lists: order, sync between devices, sort and filters leave the pinned ones alone.
mod common;
use client_core::ListKind;
use common::*;

fn ids(v: &[proto::ConversationSummary]) -> Vec<String> {
    v.iter().map(|c| c.conversation.id.clone()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn pinned_conversations_stay_on_top_in_the_users_order_and_follow_the_user() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, _bob) = s.user("bob").await;
    let c1 = make_channel(&s, "alpha", &[&a_id, &b_id]).await;
    let c2 = make_channel(&s, "beta", &[&a_id, &b_id]).await;
    let c3 = make_channel(&s, "gamma", &[&a_id, &b_id]).await;
    let c4 = make_channel(&s, "delta", &[&a_id, &b_id]).await;
    wait_conv(&alice, &c4).await;
    let alice2 = s.new_client("alice-phone");
    alice2.login(&s.url, "alice", "password-123", None).await.unwrap();
    alice2.start();
    wait_online(&alice2).await;
    let mine = [c1.clone(), c2.clone(), c3.clone(), c4.clone()];
    let chans = |c: &client_core::Client, alpha: bool, unread: bool, muted: bool| {
        let l = c.phone_list(ListKind::Channels, alpha, unread, muted);
        (ids(&l.pinned).into_iter().filter(|i| mine.contains(i)).collect::<Vec<_>>(), ids(&l.others).into_iter().filter(|i| mine.contains(i)).collect::<Vec<_>>())
    };

    // Nothing pinned: everything is "others", A-Z sorts by name.
    let (p, o) = chans(&alice, true, false, false);
    assert!(p.is_empty());
    assert_eq!(o, vec![c1.clone(), c2.clone(), c4.clone(), c3.clone()], "alpha, beta, delta, gamma");

    // Pin gamma, then beta: the newest pin goes first.
    alice.pin_conversation(&c3).await.unwrap();
    alice.pin_conversation(&c2).await.unwrap();
    let (p, o) = chans(&alice, false, false, false);
    assert_eq!(p, vec![c2.clone(), c3.clone()]);
    assert!(!o.contains(&c2) && !o.contains(&c3));
    wait_until("pins on device 2", || chans(&alice2, false, false, false).0 == vec![c2.clone(), c3.clone()]).await;

    // Sort only moves the unpinned ones; the pinned order stays.
    let (p, o) = chans(&alice, true, false, false);
    assert_eq!(p, vec![c2.clone(), c3.clone()]);
    assert_eq!(o, vec![c1.clone(), c4.clone()]);

    // Filters narrow the unpinned ones only: nothing is unread in these channels (the first message of the test
    // server is the creator's), muting one hides it from the unpinned part but never a pinned one.
    alice.mute(&c1, "forever", 0).await.unwrap();
    alice.mute(&c2, "forever", 0).await.unwrap();
    let (p, o) = chans(&alice, false, false, true);
    assert_eq!(p, vec![c2.clone(), c3.clone()], "a muted pinned conversation stays");
    assert!(!o.contains(&c1), "a muted unpinned one is hidden");

    // Move pinned conversations: down then up, and the ends do nothing.
    assert!(alice.move_pinned(&c2, 1).await.unwrap());
    assert_eq!(chans(&alice, false, false, false).0, vec![c3.clone(), c2.clone()]);
    wait_until("move on device 2", || chans(&alice2, false, false, false).0 == vec![c3.clone(), c2.clone()]).await;
    assert!(!alice.move_pinned(&c2, 1).await.unwrap());
    assert!(alice.move_pinned(&c2, -1).await.unwrap());
    assert_eq!(chans(&alice, false, false, false).0, vec![c2.clone(), c3.clone()]);

    // Drag and drop result: a new order for the pinned ones of the tab.
    alice.reorder_pinned(&[c3.clone(), c2.clone()]).await.unwrap();
    assert_eq!(chans(&alice, false, false, false).0, vec![c3.clone(), c2.clone()]);

    // Unpin: back among the others.
    alice.unpin_conversation(&c3).await.unwrap();
    let (p, o) = chans(&alice, false, false, false);
    assert_eq!(p, vec![c2.clone()]);
    assert!(o.contains(&c3));
    wait_until("unpin on device 2", || chans(&alice2, false, false, false).0 == vec![c2.clone()]).await;

    // The other user's lists are their own.
    assert!(chans(&_bob, false, false, false).0.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn dms_and_channels_are_separate_lists_that_share_one_pinned_order() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, _bob) = s.user("bob").await;
    let ch = make_channel(&s, "general", &[&a_id, &b_id]).await;
    wait_conv(&alice, &ch).await;
    let dm = alice.open_dm(&b_id).await.unwrap();
    alice.pin_conversation(&ch).await.unwrap();
    alice.pin_conversation(&dm).await.unwrap();
    let d = alice.phone_list(ListKind::Dms, false, false, false);
    let c = alice.phone_list(ListKind::Channels, false, false, false);
    assert!(ids(&d.pinned).contains(&dm) && !ids(&d.pinned).contains(&ch));
    assert!(ids(&c.pinned).contains(&ch) && !ids(&c.pinned).contains(&dm));
    // Re-ordering the DMs leaves the channel where it was in the shared order.
    let before = alice.pinned_ids();
    alice.reorder_pinned(&ids(&d.pinned)).await.unwrap();
    assert_eq!(alice.pinned_ids(), before);
}

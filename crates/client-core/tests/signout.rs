//! Self-service sign-out is gated by the admin's "Privacy options" (Allow) setting, on the server and in the client.
mod common;
use common::*;
use proto::*;

async fn second_device(s: &TestServer, name: &str) -> client_core::Client {
    let c = s.new_client(&format!("{name}-2"));
    c.login(&s.url, name, "password-123", None).await.unwrap();
    c.start();
    wait_online(&c).await;
    c
}

/// Subscribes first so the SignedOut event cannot be missed, then returns its code once awaited.
async fn watch(c: &client_core::Client) -> tokio::task::JoinHandle<(Option<ErrorCode>, String)> {
    let c = c.clone();
    let h = tokio::spawn(async move { wait_signed_out(&c).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    h
}

async fn set_allow(s: &TestServer, id: &str, on: bool) {
    s.admin.update_user(id, &UpdateUserRequest { privacy_controls_enabled: Some(on), ..Default::default() }).await.unwrap();
}

#[tokio::test]
async fn disallow_refuses_every_self_service_sign_out_and_keeps_sessions() {
    let s = TestServer::start().await;
    let (_id, alice) = s.user("alice").await;
    let laptop = second_device(&s, "alice").await;
    assert!(!alice.can_sign_out());
    assert_eq!(alice.logout().await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    assert_eq!(alice.logout_all().await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    assert_eq!(alice.revoke_other_sessions().await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    let list = alice.sessions().await.unwrap();
    assert_eq!(list.len(), 2);
    let other = list.iter().find(|x| !x.current).unwrap().id.clone();
    let own = list.iter().find(|x| x.current).unwrap().id.clone();
    let e = alice.revoke_session(&other).await.unwrap_err();
    assert_eq!(e.code(), Some(ErrorCode::Forbidden));
    assert!(e.to_string().contains("sign-out is not allowed for this account"), "{e}");
    assert_eq!(alice.revoke_session(&own).await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    // Nothing was wiped or ended.
    assert!(alice.signed_in() && laptop.signed_in());
    assert_eq!(alice.sessions().await.unwrap().len(), 2);
    laptop.refresh_me().await.unwrap();
    // The raw endpoints are refused for a modified client too.
    let api = alice.api().unwrap();
    for path in ["/logout", "/sessions/revoke-all", "/sessions/revoke-others"] {
        let r: Result<serde_json::Value, _> = api.post(path, &serde_json::json!({})).await;
        assert_eq!(r.unwrap_err().code(), Some(ErrorCode::Forbidden), "{path}");
    }
}

#[tokio::test]
async fn allow_enables_sign_out_and_logout_all_ends_every_device() {
    let s = TestServer::start().await;
    let (id, alice) = s.user("alice").await;
    let laptop = second_device(&s, "alice").await;
    s.allow_signout(&id, &alice).await;
    let waiter = {
        let l = laptop.clone();
        tokio::spawn(async move { wait_signed_out(&l).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    alice.logout_all().await.unwrap();
    assert!(!alice.signed_in());
    let (code, _) = waiter.await.unwrap();
    assert_eq!(code, Some(ErrorCode::SessionRevoked));
    // Both sessions are gone on the server.
    let list = s.admin.sessions().await.unwrap();
    assert!(list.iter().all(|x| x.user_id != id || x.revoked), "{list:?}");

    // Plain sign-out of one device.
    let again = second_device(&s, "alice").await;
    let keep = second_device(&s, "alice").await;
    again.logout().await.unwrap();
    assert!(!again.signed_in());
    keep.refresh_me().await.unwrap();
}

#[tokio::test]
async fn flipping_the_flag_is_followed_live_via_me_update() {
    let s = TestServer::start().await;
    let (id, alice) = s.user("alice").await;
    assert!(!alice.can_sign_out());
    set_allow(&s, &id, true).await;
    wait_until("allowed", || alice.can_sign_out()).await;
    set_allow(&s, &id, false).await;
    wait_until("disallowed again", || !alice.can_sign_out()).await;
    assert_eq!(alice.logout().await.unwrap_err().code(), Some(ErrorCode::Forbidden));
    assert!(alice.signed_in());
}

#[tokio::test]
async fn admins_are_never_locked_out_and_admin_panel_ends_any_session() {
    let s = TestServer::start().await;
    let (bob_id, bob) = s.user("bob").await;
    s.admin.set_role(&bob_id, Role::Admin).await.unwrap();
    wait_until("bob admin", || bob.me().map(|m| m.user.role == Role::Admin).unwrap_or(false)).await;
    assert!(!bob.me().unwrap().privacy_controls_enabled);
    assert!(bob.can_sign_out());
    let phone = second_device(&s, "bob").await;
    bob.revoke_other_sessions().await.unwrap();
    wait_until("phone signed out", || !phone.signed_in()).await;
    bob.logout().await.unwrap();
    assert!(!bob.signed_in());

    // An admin may end a member's sessions from the panel even without Allow.
    let (carol_id, carol) = s.user("carol").await;
    assert!(!carol.can_sign_out());
    let w = watch(&carol).await;
    s.admin.revoke_user_sessions(&carol_id).await.unwrap();
    let (code, _) = w.await.unwrap();
    assert_eq!(code, Some(ErrorCode::SessionRevoked));
}

#[tokio::test]
async fn forced_wipes_still_work_without_allow() {
    let s = TestServer::start().await;
    let (a_id, alice) = s.user("alice").await;
    let (b_id, bob) = s.user("bob").await;
    let (c_id, carol) = s.user("carol").await;
    let w = watch(&alice).await;
    s.admin.set_disabled(&a_id, true).await.unwrap();
    let (code, _) = w.await.unwrap();
    assert_eq!(code, Some(ErrorCode::AccountDisabled));
    let w = watch(&bob).await;
    s.admin.reset_password(&b_id, Some("brand-new-pass")).await.unwrap();
    let (code, _) = w.await.unwrap();
    assert_eq!(code, Some(ErrorCode::PasswordReset));
    let w = watch(&carol).await;
    s.admin.clear_history(&c_id).await.unwrap();
    let (code, _) = w.await.unwrap();
    assert_eq!(code, Some(ErrorCode::HistoryCleared));
}

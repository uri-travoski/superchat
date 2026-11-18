//! Role management: `PUT /admin/users/{id}/role` (owner only), its rules, the immediate effect on live
//! sessions, the event that updates the user's apps, and per-user backup/restore consistency.
mod common;
use client_core::admin::AdminClient;
use common::*;
use proto::*;

async fn code_of<T: std::fmt::Debug>(r: client_core::error::Result<T>) -> ErrorCode {
    r.expect_err("expected an error").code().expect("an API error code")
}

/// Whether the signed-in account may use the admin API (its role is read on every request).
async fn can_admin(a: &AdminClient) -> bool {
    a.users().await.is_ok()
}

#[tokio::test]
async fn owner_changes_roles_and_sessions_follow_immediately() {
    let s = TestServer::start().await;
    let (bob_id, bob) = s.user("bob").await;
    let bob_api = AdminClient::login(&s.url, "bob", "password-123").await.unwrap();
    assert_eq!(bob.me().unwrap().user.role, Role::Member);
    assert!(!can_admin(&bob_api).await);
    let ch = s.admin.create_channel("ops", &[]).await.unwrap();
    assert_eq!(code_of(bob_api.create_channel("mine", &[]).await).await, ErrorCode::Forbidden);

    // Member to admin: the already-open session gets the power at once, no new login.
    let u = s.admin.set_role(&bob_id, Role::Admin).await.unwrap();
    assert_eq!(u.user.role, Role::Admin);
    assert!(can_admin(&bob_api).await, "the same token is an admin right away");
    bob_api.create_channel("mine", &[]).await.expect("admins create channels");
    // The open app learns about it from an event: /me refreshes without a restart.
    wait_until("app sees admin role", || bob.me().map(|m| m.user.role == Role::Admin).unwrap_or(false)).await;
    let _ = ch;

    // Admin to member: powers vanish at once, the app follows.
    s.admin.set_role(&bob_id, Role::Member).await.unwrap();
    assert!(!can_admin(&bob_api).await);
    assert_eq!(code_of(bob_api.create_channel("again", &[]).await).await, ErrorCode::Forbidden);
    wait_until("app sees member role", || bob.me().map(|m| m.user.role == Role::Member).unwrap_or(false)).await;
    // The role is also what every other user sees.
    let (_, alice) = s.user("alice").await;
    s.admin.set_role(&bob_id, Role::Admin).await.unwrap();
    wait_until("others see the role", || alice.user(&bob_id).map(|u| u.role == Role::Admin).unwrap_or(false)).await;
    // Setting the same role again is a harmless no-op that returns the user.
    assert_eq!(s.admin.set_role(&bob_id, Role::Admin).await.unwrap().user.role, Role::Admin);
}

#[tokio::test]
async fn only_an_owner_changes_roles_and_never_their_own() {
    let s = TestServer::start().await;
    let (bob_id, _bob) = s.user("bob").await;
    let (carol_id, _carol) = s.user("carol").await;
    s.admin.set_role(&bob_id, Role::Admin).await.unwrap();
    let bob_api = AdminClient::login(&s.url, "bob", "password-123").await.unwrap();
    // An admin may use the admin panel but not change roles (not even to demote or promote a member).
    assert_eq!(code_of(bob_api.set_role(&carol_id, Role::Admin).await).await, ErrorCode::Forbidden);
    assert_eq!(code_of(bob_api.set_role(&carol_id, Role::Member).await).await, ErrorCode::Forbidden);
    assert_eq!(code_of(bob_api.set_role(&bob_id, Role::Owner).await).await, ErrorCode::Forbidden);
    // A member is refused as well.
    let carol_api = AdminClient::login(&s.url, "carol", "password-123").await.unwrap();
    assert_eq!(code_of(carol_api.set_role(&carol_id, Role::Admin).await).await, ErrorCode::Forbidden);
    // The owner cannot change their own role.
    let me = s.admin.users().await.unwrap().into_iter().find(|u| u.user.username == "admin").unwrap();
    assert_eq!(code_of(s.admin.set_role(&me.user.id, Role::Member).await).await, ErrorCode::Forbidden);
    assert_eq!(code_of(s.admin.set_role(&me.user.id, Role::Admin).await).await, ErrorCode::Forbidden);
    // Unknown user and invalid role are clean errors.
    assert_eq!(code_of(s.admin.set_role("nope", Role::Admin).await).await, ErrorCode::NotFound);
    let bad: client_core::error::Result<AdminUser> = s.admin.call(reqwest::Method::PUT, &format!("/admin/users/{carol_id}/role"), Some(serde_json::json!({"role": "king"}))).await;
    assert!(bad.is_err());
    // Nothing above changed anything.
    let users = s.admin.users().await.unwrap();
    let role = |n: &str| users.iter().find(|u| u.user.username == n).unwrap().user.role;
    assert_eq!((role("admin"), role("bob"), role("carol")), (Role::Owner, Role::Admin, Role::Member));
}

#[tokio::test]
async fn owners_can_be_added_and_one_owner_always_remains() {
    let s = TestServer::start().await;
    let (bob_id, _bob) = s.user("bob").await;
    let (carol_id, _carol) = s.user("carol").await;
    let admin_id = s.admin.users().await.unwrap().into_iter().find(|u| u.user.username == "admin").unwrap().user.id;
    // Promote bob to owner: he then manages roles himself.
    assert_eq!(s.admin.set_role(&bob_id, Role::Owner).await.unwrap().user.role, Role::Owner);
    let bob_api = AdminClient::login(&s.url, "bob", "password-123").await.unwrap();
    assert_eq!(bob_api.set_role(&carol_id, Role::Admin).await.unwrap().user.role, Role::Admin);
    // He can transfer ownership by demoting the first owner (he is not the last one) ...
    assert_eq!(bob_api.set_role(&admin_id, Role::Admin).await.unwrap().user.role, Role::Admin);
    // ... and the demoted former owner has lost the right to change roles at once.
    assert_eq!(code_of(s.admin.set_role(&carol_id, Role::Member).await).await, ErrorCode::Forbidden);
    // The only remaining owner cannot demote themselves.
    assert_eq!(code_of(bob_api.set_role(&bob_id, Role::Member).await).await, ErrorCode::Forbidden);
    let owners = bob_api.users().await.unwrap().into_iter().filter(|u| u.user.role == Role::Owner).count();
    assert_eq!(owners, 1);
}

#[tokio::test]
async fn bots_stay_members_and_disabled_users_can_be_changed() {
    let s = TestServer::start().await;
    let bot = s.admin.create_bot("tester_bot", "Tester", "t", &[]).await.unwrap();
    assert_eq!(code_of(s.admin.set_role(&bot.user.id, Role::Admin).await).await, ErrorCode::Validation);
    assert_eq!(code_of(s.admin.set_role(&bot.user.id, Role::Owner).await).await, ErrorCode::Validation);
    assert_eq!(s.admin.set_role(&bot.user.id, Role::Member).await.unwrap().user.role, Role::Member);

    let (dan_id, dan) = s.user("dan").await;
    s.admin.set_disabled(&dan_id, true).await.unwrap();
    wait_signed_out(&dan).await;
    assert_eq!(s.admin.set_role(&dan_id, Role::Admin).await.unwrap().user.role, Role::Admin);
    assert!(s.admin.users().await.unwrap().iter().any(|u| u.user.id == dan_id && u.user.disabled && u.user.role == Role::Admin));
    // After re-enabling, he signs in with the new role.
    s.admin.set_disabled(&dan_id, false).await.unwrap();
    let dan2 = s.new_client("dan2");
    dan2.login(&s.url, "dan", "password-123", None).await.unwrap();
    assert_eq!(dan2.me().unwrap().user.role, Role::Admin);
    // Deleted accounts cannot be changed.
    let (erin_id, _erin) = s.user("erin").await;
    s.admin.delete_user(&erin_id).await.unwrap();
    assert_eq!(code_of(s.admin.set_role(&erin_id, Role::Admin).await).await, ErrorCode::Validation);
}

#[tokio::test]
async fn per_user_backup_and_restore_keep_roles_consistent() {
    let s = TestServer::start().await;
    let (bob_id, _bob) = s.user("bob").await;
    s.admin.set_role(&bob_id, Role::Admin).await.unwrap();
    let b1 = s.admin.backup_user(&bob_id).await.unwrap();
    // Restoring over a live account never touches its current role (the role is not part of "replace").
    s.admin.set_role(&bob_id, Role::Member).await.unwrap();
    s.admin.restore(&RestoreRequest { backup_id: b1.id.clone(), user_ids: vec![bob_id.clone()], all: false, mode: Some("replace".into()) }).await.unwrap();
    let role = |us: &[AdminUser]| us.iter().find(|u| u.user.id == bob_id).unwrap().user.role;
    assert_eq!(role(&s.admin.users().await.unwrap()), Role::Member);
    // A deleted account comes back with the role it had in the backup ...
    s.admin.set_role(&bob_id, Role::Admin).await.unwrap();
    s.admin.delete_user(&bob_id).await.unwrap();
    let bk = s.admin.backups().await.unwrap().into_iter().find(|b| b.user_id.as_deref() == Some(bob_id.as_str())).expect("deletion backup");
    s.admin.restore(&RestoreRequest { backup_id: bk.id, user_ids: vec![bob_id.clone()], all: false, mode: None }).await.unwrap();
    assert_eq!(role(&s.admin.users().await.unwrap()), Role::Admin);
    // ... and a backup that says "owner" never creates a second owner by accident: it comes back as admin.
    s.admin.set_role(&bob_id, Role::Owner).await.unwrap();
    let b2 = s.admin.backup_user(&bob_id).await.unwrap();
    s.admin.set_role(&bob_id, Role::Member).await.unwrap();
    s.admin.delete_user(&bob_id).await.unwrap();
    s.admin.restore(&RestoreRequest { backup_id: b2.id, user_ids: vec![bob_id.clone()], all: false, mode: None }).await.unwrap();
    assert_eq!(role(&s.admin.users().await.unwrap()), Role::Admin);
}

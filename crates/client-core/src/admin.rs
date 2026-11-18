//! Thin typed wrapper over the admin REST API (used by tests, scripts and the CLI).

use crate::error::{ClientError, Result};
use proto::*;
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub struct AdminClient {
    base: String,
    http: reqwest::Client,
    pub token: String,
}

impl AdminClient {
    /// Signs in as an admin or owner.
    pub async fn login(server: &str, username: &str, password: &str) -> Result<AdminClient> {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build().map_err(|e| ClientError::Local(e.to_string()))?;
        let base = server.trim_end_matches('/').to_string();
        let resp = http
            .post(format!("{base}{API_PREFIX}/login"))
            .json(&LoginRequest { username: username.into(), password: password.into(), device_name: "admin-client".into(), client_version: crate::CLIENT_VERSION.into() })
            .send()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(parse_err(resp).await);
        }
        let l: LoginResponse = resp.json().await.map_err(|e| ClientError::Network(e.to_string()))?;
        Ok(AdminClient { base, http, token: l.tokens.access_token })
    }

    pub async fn setup(server: &str, req: &SetupRequest) -> Result<AdminClient> {
        let http = reqwest::Client::new();
        let base = server.trim_end_matches('/').to_string();
        let resp = http.post(format!("{base}{API_PREFIX}/setup")).json(req).send().await.map_err(|e| ClientError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(parse_err(resp).await);
        }
        let l: LoginResponse = resp.json().await.map_err(|e| ClientError::Network(e.to_string()))?;
        Ok(AdminClient { base, http, token: l.tokens.access_token })
    }

    pub async fn call<T: DeserializeOwned>(&self, method: Method, path: &str, body: Option<serde_json::Value>) -> Result<T> {
        let mut rb = self.http.request(method, format!("{}{}{}", self.base, API_PREFIX, path)).bearer_auth(&self.token);
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        let resp = rb.send().await.map_err(|e| ClientError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(parse_err(resp).await);
        }
        resp.json().await.map_err(|e| ClientError::Network(e.to_string()))
    }

    async fn post<T: DeserializeOwned, B: Serialize>(&self, path: &str, b: &B) -> Result<T> {
        self.call(Method::POST, path, Some(serde_json::to_value(b)?)).await
    }

    pub async fn create_user(&self, username: &str, display: &str, password: Option<&str>) -> Result<CreateUserResponse> {
        self.post(
            "/admin/users",
            &CreateUserRequest {
                username: username.into(),
                display_name: display.into(),
                password: password.map(str::to_string),
                role: None,
                email: None,
                create_login_link: None,
            },
        )
        .await
    }

    pub async fn users(&self) -> Result<Vec<AdminUser>> {
        self.call(Method::GET, "/admin/users", None).await
    }

    /// Owner only: sets a user's role (`PUT /admin/users/{id}/role`).
    pub async fn set_role(&self, id: &str, role: Role) -> Result<AdminUser> {
        self.call(Method::PUT, &format!("/admin/users/{id}/role"), Some(serde_json::to_value(SetRoleRequest { role })?)).await
    }

    pub async fn set_disabled(&self, id: &str, disabled: bool) -> Result<AdminUser> {
        self.call(Method::PATCH, &format!("/admin/users/{id}"), Some(serde_json::json!({"disabled": disabled}))).await
    }

    pub async fn update_user(&self, id: &str, req: &UpdateUserRequest) -> Result<AdminUser> {
        self.call(Method::PATCH, &format!("/admin/users/{id}"), Some(serde_json::to_value(req)?)).await
    }

    pub async fn reset_password(&self, id: &str, new_password: Option<&str>) -> Result<ResetPasswordResponse> {
        self.post(&format!("/admin/users/{id}/reset-password"), &ResetPasswordRequest { new_password: new_password.map(str::to_string) }).await
    }

    pub async fn login_link(&self, id: &str) -> Result<LoginLink> {
        self.post(&format!("/admin/users/{id}/login-link"), &serde_json::json!({})).await
    }

    pub async fn clear_history(&self, id: &str) -> Result<ClearHistoryResponse> {
        self.post(&format!("/admin/users/{id}/clear-history"), &serde_json::json!({})).await
    }

    pub async fn delete_user(&self, id: &str) -> Result<ClearHistoryResponse> {
        self.call(Method::DELETE, &format!("/admin/users/{id}"), None).await
    }

    pub async fn backup_user(&self, id: &str) -> Result<BackupInfo> {
        self.post(&format!("/admin/users/{id}/backup"), &serde_json::json!({})).await
    }

    pub async fn delete_range(&self, id: &str, req: &DateRangeDeleteRequest) -> Result<ClearHistoryResponse> {
        self.post(&format!("/admin/users/{id}/delete-range"), req).await
    }

    pub async fn backups(&self) -> Result<Vec<BackupInfo>> {
        self.call(Method::GET, "/admin/backups", None).await
    }

    pub async fn create_channel(&self, name: &str, members: &[String]) -> Result<ConversationSummary> {
        self.post("/conversations", &CreateConversation { kind: ConversationKind::Channel, name: name.into(), topic: None, members: members.to_vec() }).await
    }

    pub async fn create_group(&self, name: &str, members: &[String]) -> Result<ConversationSummary> {
        self.post("/conversations", &CreateConversation { kind: ConversationKind::GroupDm, name: name.into(), topic: None, members: members.to_vec() }).await
    }

    pub async fn add_members(&self, conv: &str, users: &[String]) -> Result<Conversation> {
        self.post(&format!("/admin/conversations/{conv}/members"), &MembersRequest { user_ids: users.to_vec() }).await
    }

    pub async fn conversations(&self) -> Result<Vec<AdminConversation>> {
        self.call(Method::GET, "/admin/conversations", None).await
    }

    pub async fn storage(&self) -> Result<StorageInfo> {
        self.call(Method::GET, "/admin/storage", None).await
    }

    pub async fn test_storage(&self, cfg: &StorageConfig) -> Result<StorageTestResponse> {
        self.post("/admin/storage/test", cfg).await
    }

    pub async fn set_storage(&self, cfg: &StorageConfig) -> Result<StorageInfo> {
        self.call(Method::PUT, "/admin/storage", Some(serde_json::to_value(cfg)?)).await
    }

    pub async fn migrate_storage(&self, target: &StorageConfig, delete_source: bool) -> Result<JobStatus> {
        self.post("/admin/storage/migrate", &StorageMigrateRequest { target: target.clone(), delete_source }).await
    }

    pub async fn verify_storage(&self, deep: bool) -> Result<JobStatus> {
        self.post("/admin/storage/verify", &StorageVerifyRequest { deep }).await
    }

    pub async fn job(&self, id: &str) -> Result<JobStatus> {
        self.call(Method::GET, &format!("/admin/jobs/{id}"), None).await
    }

    /// Polls a job until it is no longer running.
    pub async fn wait_job(&self, id: &str) -> Result<JobStatus> {
        for _ in 0..600 {
            let j = self.job(id).await?;
            if j.state != "running" {
                return Ok(j);
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        Err(ClientError::Local("job timed out".into()))
    }

    pub async fn remove_file(&self, id: &str) -> Result<serde_json::Value> {
        self.call(Method::DELETE, &format!("/admin/files/{id}"), None).await
    }

    pub async fn sessions(&self) -> Result<Vec<SessionInfo>> {
        self.call(Method::GET, "/admin/sessions", None).await
    }

    pub async fn revoke_user_sessions(&self, user_id: &str) -> Result<serde_json::Value> {
        self.post(&format!("/admin/users/{user_id}/revoke-sessions"), &serde_json::json!({})).await
    }

    // ---- backups (SV-20)

    pub async fn backup_settings(&self) -> Result<BackupSettings> {
        self.call(Method::GET, "/admin/backups/settings", None).await
    }

    pub async fn put_backup_settings(&self, req: &BackupSettingsUpdate) -> Result<BackupSettings> {
        self.call(Method::PUT, "/admin/backups/settings", Some(serde_json::to_value(req)?)).await
    }

    pub async fn test_backup_target(&self) -> Result<StorageTestResponse> {
        self.post("/admin/backups/test-target", &serde_json::json!({})).await
    }

    pub async fn backup_now(&self) -> Result<BackupInfo> {
        self.post("/admin/backups/full", &serde_json::json!({})).await
    }

    pub async fn restore(&self, req: &RestoreRequest) -> Result<RestoreResponse> {
        self.post("/admin/restore", req).await
    }

    /// Fresh-server restore (no authentication needed while the server has no users).
    pub async fn fresh_restore(server: &str, req: &FreshRestoreRequest) -> Result<FreshRestoreResponse> {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(600)).build().map_err(|e| ClientError::Local(e.to_string()))?;
        let resp = http.post(format!("{}{}/restore", server.trim_end_matches('/'), API_PREFIX)).json(req).send().await.map_err(|e| ClientError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(parse_err(resp).await);
        }
        resp.json().await.map_err(|e| ClientError::Network(e.to_string()))
    }

    // ---- viewer and bulk delete

    pub async fn viewer_conversations(&self, user_id: Option<&str>, kind: Option<&str>, q: Option<&str>) -> Result<Vec<ViewerConversation>> {
        let mut p = vec![];
        if let Some(u) = user_id {
            p.push(format!("user_id={u}"));
        }
        if let Some(k) = kind {
            p.push(format!("kind={k}"));
        }
        if let Some(q) = q {
            p.push(format!("q={}", url::form_urlencoded::byte_serialize(q.as_bytes()).collect::<String>()));
        }
        self.call(Method::GET, &format!("/admin/viewer/conversations?{}", p.join("&")), None).await
    }

    pub async fn viewer_messages(&self, conv: &str, query: &str) -> Result<ViewerHistory> {
        self.call(Method::GET, &format!("/admin/viewer/conversations/{conv}/messages?{query}"), None).await
    }

    pub async fn viewer_search(&self, query: &str) -> Result<Vec<SearchHit>> {
        self.call(Method::GET, &format!("/admin/viewer/search?{query}"), None).await
    }

    pub async fn delete_messages(&self, req: &AdminDeleteMessagesRequest) -> Result<AdminDeleteMessagesResponse> {
        self.post("/admin/messages/delete", req).await
    }

    // ---- groups, push

    pub async fn create_user_group(&self, handle: &str, name: &str, members: &[String]) -> Result<UserGroup> {
        self.post("/admin/groups", &UserGroupRequest { handle: Some(handle.into()), name: Some(name.into()), member_ids: Some(members.to_vec()) }).await
    }

    pub async fn push_settings(&self) -> Result<PushSettings> {
        self.call(Method::GET, "/admin/push", None).await
    }

    pub async fn put_push_settings(&self, s: &PushSettings) -> Result<PushSettings> {
        self.call(Method::PUT, "/admin/push", Some(serde_json::to_value(s)?)).await
    }

    // ---- bots, webhooks, AI (phase 3)

    pub async fn bots(&self) -> Result<Vec<AdminBot>> {
        self.call(Method::GET, "/admin/bots", None).await
    }

    pub async fn create_bot(&self, username: &str, display: &str, description: &str, conv_ids: &[String]) -> Result<AdminBot> {
        self.post("/admin/bots", &CreateBotRequest { username: username.into(), display_name: display.into(), description: description.into(), conv_ids: conv_ids.to_vec() }).await
    }

    pub async fn update_bot(&self, id: &str, req: &UpdateBotRequest) -> Result<AdminBot> {
        self.call(Method::PATCH, &format!("/admin/bots/{id}"), Some(serde_json::to_value(req)?)).await
    }

    pub async fn regenerate_bot_token(&self, id: &str) -> Result<AdminBot> {
        self.call(Method::POST, &format!("/admin/bots/{id}/token"), Some(serde_json::json!({}))).await
    }

    pub async fn delete_bot(&self, id: &str) -> Result<serde_json::Value> {
        self.call(Method::DELETE, &format!("/admin/bots/{id}"), None).await
    }

    pub async fn add_bot_to(&self, id: &str, conv: &str) -> Result<AdminBot> {
        self.call(Method::PUT, &format!("/admin/bots/{id}/conversations/{conv}"), Some(serde_json::json!({}))).await
    }

    pub async fn remove_bot_from(&self, id: &str, conv: &str) -> Result<AdminBot> {
        self.call(Method::DELETE, &format!("/admin/bots/{id}/conversations/{conv}"), None).await
    }

    pub async fn webhooks(&self) -> Result<WebhooksOverview> {
        self.call(Method::GET, "/admin/webhooks", None).await
    }

    pub async fn create_incoming_webhook(&self, name: &str, conv_id: &str) -> Result<IncomingWebhook> {
        self.post("/admin/webhooks/incoming", &CreateIncomingWebhook { name: name.into(), conv_id: conv_id.into() }).await
    }

    pub async fn create_outgoing_webhook(&self, req: &CreateOutgoingWebhook) -> Result<OutgoingWebhook> {
        self.post("/admin/webhooks/outgoing", req).await
    }

    pub async fn update_outgoing_webhook(&self, id: &str, req: &UpdateOutgoingWebhook) -> Result<OutgoingWebhook> {
        self.call(Method::PATCH, &format!("/admin/webhooks/outgoing/{id}"), Some(serde_json::to_value(req)?)).await
    }

    pub async fn test_outgoing_webhook(&self, id: &str) -> Result<OutgoingWebhook> {
        self.call(Method::POST, &format!("/admin/webhooks/outgoing/{id}/test"), Some(serde_json::json!({}))).await
    }

    pub async fn get_public_url(&self) -> Result<PublicUrlSettings> {
        self.call(Method::GET, "/admin/public-url", None).await
    }

    pub async fn set_public_url(&self, url: Option<&str>) -> Result<PublicUrlSettings> {
        self.call(Method::PUT, "/admin/public-url", Some(serde_json::to_value(&PublicUrlUpdate { url: url.map(str::to_string) })?)).await
    }

    pub async fn ai_overview(&self) -> Result<AiOverview> {
        self.call(Method::GET, "/admin/ai", None).await
    }

    pub async fn add_ai_provider(&self, i: &AiProviderInput) -> Result<AiProvider> {
        self.call(Method::POST, "/admin/ai/providers", Some(serde_json::to_value(i)?)).await
    }

    pub async fn update_ai_provider(&self, id: &str, u: &AiProviderUpdate) -> Result<AiProvider> {
        self.call(Method::PATCH, &format!("/admin/ai/providers/{id}"), Some(serde_json::to_value(u)?)).await
    }

    pub async fn delete_ai_provider(&self, id: &str) -> Result<AiOverview> {
        self.call(Method::DELETE, &format!("/admin/ai/providers/{id}"), None).await
    }

    pub async fn test_ai_provider(&self, id: &str) -> Result<AiTestResponse> {
        self.call(Method::POST, &format!("/admin/ai/providers/{id}/test"), Some(serde_json::json!({}))).await
    }

    pub async fn put_ai_defaults(&self, u: &AiDefaultsUpdate) -> Result<AiOverview> {
        self.call(Method::PUT, "/admin/ai/defaults", Some(serde_json::to_value(u)?)).await
    }
}

async fn parse_err(resp: reqwest::Response) -> ClientError {
    match resp.json::<ErrorBody>().await {
        Ok(b) => ClientError::Api { code: b.error.code, message: b.error.message },
        Err(e) => ClientError::Network(e.to_string()),
    }
}

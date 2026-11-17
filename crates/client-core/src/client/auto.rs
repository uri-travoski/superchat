//! Phase 3 client API: scheduled messages, form responses, bot profiles, AI search and recap.

use super::*;
use std::collections::BTreeMap;

const SCHEDULED_KEY: &str = "scheduled";

impl Client {
    // ------------------------------------------------------------------ scheduled (SV-10, M-23, M-24)

    /// Pending messages first (soonest first), then what was sent, cancelled or failed recently.
    pub fn scheduled(&self) -> Vec<ScheduledMessage> {
        let mut v: Vec<ScheduledMessage> =
            self.inner.store.kv_get(SCHEDULED_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        v.sort_by_key(|m| (m.state != ScheduledState::Pending, m.due_ms));
        v
    }

    pub fn scheduled_pending(&self, conv_id: Option<&str>) -> Vec<ScheduledMessage> {
        self.scheduled().into_iter().filter(|m| m.state == ScheduledState::Pending && conv_id.map(|c| c == m.conv_id).unwrap_or(true)).collect()
    }

    pub async fn refresh_scheduled(&self) -> Result<Vec<ScheduledMessage>> {
        let api = self.api()?;
        let v: Vec<ScheduledMessage> = api.get("/scheduled").await?;
        self.inner.store.kv_set(SCHEDULED_KEY, &serde_json::to_string(&v)?)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![], users: false, me: false });
        Ok(self.scheduled())
    }

    pub async fn schedule_message(&self, conv_id: &str, body: &str, due_ms: i64, thread_root: Option<&str>) -> Result<ScheduledMessage> {
        let api = self.api()?;
        let item: ScheduledMessage = api
            .post(&format!("/conversations/{conv_id}/scheduled"), &ScheduleRequest { body: body.to_string(), due_ms, thread_root: thread_root.map(str::to_string), attachment_ids: vec![] })
            .await?;
        self.inner.store.merge_scheduled(&item)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![], users: false, me: false });
        Ok(item)
    }

    pub async fn cancel_scheduled(&self, id: &str) -> Result<ScheduledMessage> {
        let api = self.api()?;
        let item: ScheduledMessage = api.delete(&format!("/scheduled/{id}")).await?;
        self.inner.store.merge_scheduled(&item)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![], users: false, me: false });
        Ok(item)
    }

    // ------------------------------------------------------------------ forms (WF-01, A-03)

    /// Submits a form or presses a button on a bot message. Returns the updated message.
    pub async fn submit_form(&self, message_id: &str, button_id: Option<&str>, values: BTreeMap<String, serde_json::Value>) -> Result<Message> {
        let api = self.api()?;
        let m: Message = api
            .post(&format!("/messages/{message_id}/form-response"), &FormSubmit { button_id: button_id.map(str::to_string), values })
            .await?;
        self.inner.store.put_message_public(&m)?;
        self.inner.emit(ClientEvent::Changed { conv_ids: vec![m.conv_id.clone()], users: false, me: false });
        Ok(m)
    }

    pub fn i_responded(&self, m: &Message) -> bool {
        self.my_id().map(|me| m.responded.contains(&me)).unwrap_or(false)
    }

    // ------------------------------------------------------------------ bots (A-04)

    pub async fn bot_profiles(&self) -> Result<Vec<BotProfile>> {
        self.api()?.get("/bots").await
    }

    pub async fn bot_profile(&self, user_id: &str) -> Result<BotProfile> {
        self.api()?.get(&format!("/bots/{user_id}")).await
    }

    /// Presses a button or submits a form on a bot's Home tab.
    pub async fn home_submit(&self, bot_id: &str, form_id: Option<&str>, button_id: Option<&str>, values: BTreeMap<String, serde_json::Value>) -> Result<()> {
        let body = serde_json::json!({"form_id": form_id, "button_id": button_id, "values": values});
        let _: serde_json::Value = self.api()?.post(&format!("/bots/{bot_id}/home/submit"), &body).await?;
        Ok(())
    }

    // ------------------------------------------------------------------ AI (SV-21, SR-07)

    /// Re-reads the public server info (the admin may have switched AI features on or off).
    pub async fn refresh_server_meta(&self) -> Result<ServerMeta> {
        let m = self.api()?.meta().await?;
        *self.inner.server_meta.lock().unwrap_or_else(|e| e.into_inner()) = Some(m.clone());
        let _ = self.inner.store.kv_set("server_meta", &serde_json::to_string(&m)?);
        Ok(m)
    }

    pub fn ai_enabled(&self) -> bool {
        self.server_meta().map(|m| m.ai_search_enabled).unwrap_or(false)
    }

    pub async fn ai_search(&self, query: &str, answer: bool) -> Result<AiSearchResponse> {
        self.api()?.post("/ai/search", &AiSearchRequest { query: query.to_string(), answer }).await
    }

    pub async fn ai_recap(&self, conv_id: &str, since_ms: Option<i64>) -> Result<AiRecapResponse> {
        self.api()?.post("/ai/recap", &AiRecapRequest { conv_id: conv_id.to_string(), since_ms }).await
    }
}

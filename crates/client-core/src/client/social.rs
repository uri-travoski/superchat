//! 1.2 client API: membership management, Save for later with labels, the Activity feed.

use super::*;

/// A saved message ready to show: the item, the message (None while it is still being fetched) and its labels.
#[derive(Debug, Clone)]
pub struct SavedView {
    pub item: SavedItem,
    pub message: Option<Message>,
}

impl Client {
    // ------------------------------------------------------------------ membership (owners and admins)

    /// Whether the signed-in user may add and remove people in this conversation (server owners and admins).
    pub fn can_manage_members(&self, conv_id: &str) -> bool {
        let Some(me) = self.me() else { return false };
        let Some(cs) = self.inner.store.conversation(conv_id).ok().flatten() else { return false };
        if !matches!(cs.conversation.kind, ConversationKind::Channel | ConversationKind::GroupDm) {
            return false;
        }
        me.user.role.is_admin() || cs.conversation.members.iter().any(|m| m.user_id == me.user.id && m.role == "owner")
    }

    async fn refresh_conversation(&self, conv_id: &str) {
        self.inner.fetch_conversation(conv_id).await;
        self.changed(vec![conv_id.to_string()]);
    }

    pub async fn add_members(&self, conv_id: &str, user_ids: &[String]) -> Result<Conversation> {
        let c: Conversation = self.api()?.post(&format!("/conversations/{conv_id}/members"), &MembersRequest { user_ids: user_ids.to_vec() }).await?;
        self.refresh_conversation(conv_id).await;
        Ok(c)
    }

    pub async fn remove_member(&self, conv_id: &str, user_id: &str) -> Result<Conversation> {
        let c: Conversation = self.api()?.delete(&format!("/conversations/{conv_id}/members/{user_id}")).await?;
        self.refresh_conversation(conv_id).await;
        Ok(c)
    }

    // ------------------------------------------------------------------ saved items and labels

    pub fn labels(&self) -> Vec<Label> {
        self.inner.store.labels()
    }

    pub fn is_saved(&self, message_id: &str) -> bool {
        self.inner.store.saved().iter().any(|s| s.message_id == message_id)
    }

    pub fn saved_item(&self, message_id: &str) -> Option<SavedItem> {
        self.inner.store.saved().into_iter().find(|s| s.message_id == message_id)
    }

    /// Saved items newest first, without the ones in a conversation App Lock hides.
    pub fn saved_items(&self) -> Vec<SavedView> {
        self.inner
            .store
            .saved()
            .into_iter()
            .filter(|i| !self.is_hidden(&i.conv_id))
            .map(|item| {
                let message = self.inner.store.message(&item.message_id).ok().flatten();
                SavedView { item, message }
            })
            .collect()
    }

    /// Fetches the messages of saved items that are not in the cache yet. Returns how many were loaded.
    pub async fn load_saved_messages(&self) -> usize {
        let mut n = 0;
        for v in self.saved_items().into_iter().filter(|v| v.message.is_none()) {
            if let Ok(api) = self.api() {
                if let Ok(msg) = api.get::<Message>(&format!("/messages/{}", v.item.message_id)).await {
                    let _ = self.inner.store.put_messages(&[msg]);
                    n += 1;
                }
            }
        }
        if n > 0 {
            self.changed(vec![]);
        }
        n
    }

    pub async fn refresh_saved(&self) -> Result<Vec<SavedItem>> {
        let v: Vec<SavedItem> = self.api()?.get("/saved").await?;
        self.inner.store.put_saved(&v)?;
        let l: Vec<Label> = self.api()?.get("/labels").await?;
        self.inner.store.put_labels(&l)?;
        self.changed(vec![]);
        Ok(v)
    }

    fn put_saved_item(&self, item: &SavedItem) -> Result<()> {
        let mut all = self.inner.store.saved();
        all.retain(|x| x.message_id != item.message_id);
        all.insert(0, item.clone());
        all.sort_by(|a, b| b.saved_ms.cmp(&a.saved_ms));
        self.inner.store.put_saved(&all)?;
        self.changed(vec![item.conv_id.clone()]);
        Ok(())
    }

    pub async fn save_message(&self, message_id: &str, label_ids: Option<&[String]>) -> Result<SavedItem> {
        let item: SavedItem = self.api()?.put_json(&format!("/saved/{message_id}"), &SaveRequest { label_ids: label_ids.map(|l| l.to_vec()) }).await?;
        self.put_saved_item(&item)?;
        Ok(item)
    }

    pub async fn unsave_message(&self, message_id: &str) -> Result<()> {
        let _: serde_json::Value = self.api()?.delete(&format!("/saved/{message_id}")).await?;
        let mut all = self.inner.store.saved();
        let conv = all.iter().find(|x| x.message_id == message_id).map(|x| x.conv_id.clone());
        all.retain(|x| x.message_id != message_id);
        self.inner.store.put_saved(&all)?;
        self.changed(conv.into_iter().collect());
        Ok(())
    }

    pub async fn set_saved_labels(&self, message_id: &str, label_ids: &[String]) -> Result<SavedItem> {
        let item: SavedItem = self.api()?.put_json(&format!("/saved/{message_id}/labels"), &SavedLabelsRequest { label_ids: label_ids.to_vec() }).await?;
        self.put_saved_item(&item)?;
        Ok(item)
    }

    pub async fn create_label(&self, name: &str, color: &str) -> Result<Vec<Label>> {
        let v: Vec<Label> = self.api()?.post("/labels", &LabelRequest { name: Some(name.into()), color: Some(color.into()), position: None }).await?;
        self.inner.store.put_labels(&v)?;
        self.changed(vec![]);
        Ok(v)
    }

    pub async fn update_label(&self, id: &str, name: Option<&str>, color: Option<&str>) -> Result<Vec<Label>> {
        let v: Vec<Label> = self.api()?.patch(&format!("/labels/{id}"), &LabelRequest { name: name.map(str::to_string), color: color.map(str::to_string), position: None }).await?;
        self.inner.store.put_labels(&v)?;
        self.changed(vec![]);
        Ok(v)
    }

    pub async fn delete_label(&self, id: &str) -> Result<Vec<Label>> {
        let v: Vec<Label> = self.api()?.delete(&format!("/labels/{id}")).await?;
        self.inner.store.put_labels(&v)?;
        let mut saved = self.inner.store.saved();
        for s in &mut saved {
            s.label_ids.retain(|l| l != id);
        }
        self.inner.store.put_saved(&saved)?;
        self.changed(vec![]);
        Ok(v)
    }

    // ------------------------------------------------------------------ activity

    /// The cached feed (newest first) minus items in locked conversations; `unread` follows the read marker.
    pub fn activity(&self) -> Vec<ActivityItem> {
        let read = self.inner.store.activity_read_ms();
        let mut v = self.inner.store.activity();
        v.retain(|i| !self.is_hidden(&i.conv_id));
        for i in &mut v {
            i.unread = i.ts > read;
        }
        v
    }

    pub fn activity_unread(&self) -> usize {
        self.activity().iter().filter(|i| i.unread).count()
    }

    pub async fn refresh_activity(&self) -> Result<Vec<ActivityItem>> {
        let f: ActivityFeed = self.api()?.get("/activity?limit=150").await?;
        self.inner.store.put_activity(&f.items)?;
        self.inner.store.set_activity_read_ms(f.read_ms)?;
        self.changed(vec![]);
        Ok(self.activity())
    }

    pub async fn mark_activity_read(&self) -> Result<()> {
        let f: ActivityFeed = self.api()?.post("/activity/read", &serde_json::json!({})).await?;
        self.inner.store.put_activity(&f.items)?;
        self.inner.store.set_activity_read_ms(f.read_ms)?;
        self.changed(vec![]);
        Ok(())
    }
}

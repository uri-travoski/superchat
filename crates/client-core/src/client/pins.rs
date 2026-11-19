//! 1.4 phone lists: the DMs tab and the Channels tab, with pinned conversations on top.
//!
//! "Pinned" is the existing per-user favourite flag; the order of the pinned ones is the existing per-user `position`
//! (both synced to every device by the preference events, see `ConvPref`). Unpinned conversations follow the list
//! setting (recent activity or name). Pinned ones always stay on top, in the user's own order, whatever the sort or
//! the filters say: "Unread only" and "Hide muted" narrow the unpinned part only.

use super::*;

/// Which phone tab a list is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    /// Direct messages, group chats and the note to yourself.
    Dms,
    /// Channels the user belongs to.
    Channels,
}

impl ListKind {
    fn has(self, k: ConversationKind) -> bool {
        (k == ConversationKind::Channel) == (self == ListKind::Channels)
    }
}

/// One phone list: pinned first (the user's order), then the rest (sorted and filtered), then archived ones.
#[derive(Clone, Debug, Default)]
pub struct PhoneList {
    pub pinned: Vec<ConversationSummary>,
    pub others: Vec<ConversationSummary>,
    pub archived: Vec<ConversationSummary>,
}

impl Client {
    fn pin_prefs(&self) -> std::collections::HashMap<String, ConvPref> {
        self.inner.store.prefs().into_iter().map(|p| (p.conv_id.clone(), p)).collect()
    }

    /// Every pinned (favourite, not archived) conversation of both kinds, in the user's order. A pinned one without a
    /// saved place (pinned on another client that predates 1.4) goes first, newest activity first.
    pub fn pinned_ids(&self) -> Vec<String> {
        let prefs = self.pin_prefs();
        let mut v: Vec<ConversationSummary> = self.conversations().into_iter().filter(|c| !c.conversation.archived && prefs.get(&c.conversation.id).map(|p| p.favourite).unwrap_or(false)).collect();
        v.sort_by_key(|c| std::cmp::Reverse(c.conversation.last_message_ts.unwrap_or(c.conversation.created_ms)));
        v.sort_by_key(|c| prefs.get(&c.conversation.id).and_then(|p| p.position).map(|p| (1, p)).unwrap_or((0, 0)));
        v.into_iter().map(|c| c.conversation.id).collect()
    }

    /// The conversations of one phone tab. `sort_alpha`, `unread_only` and `hide_muted` apply to the unpinned part only.
    pub fn phone_list(&self, kind: ListKind, sort_alpha: bool, unread_only: bool, hide_muted: bool) -> PhoneList {
        let prefs = self.pin_prefs();
        let now = now_ms();
        let pinned_order = self.pinned_ids();
        let all: Vec<ConversationSummary> = self.conversations().into_iter().filter(|c| kind.has(c.conversation.kind)).collect();
        let mut out = PhoneList::default();
        let mut pinned: Vec<ConversationSummary> = vec![];
        for c in all {
            if c.conversation.archived {
                out.archived.push(c);
                continue;
            }
            let p = prefs.get(&c.conversation.id);
            if p.map(|p| p.favourite).unwrap_or(false) {
                pinned.push(c);
                continue;
            }
            let muted = p.map(|p| p.is_muted(now)).unwrap_or(false);
            if (!unread_only || c.unread > 0 || c.marked_unread) && (!hide_muted || !muted) {
                out.others.push(c);
            }
        }
        pinned.sort_by_key(|c| pinned_order.iter().position(|i| *i == c.conversation.id).unwrap_or(usize::MAX));
        out.pinned = pinned;
        let title = |c: &ConversationSummary| self.conversation_title(c).to_lowercase();
        let recent = |c: &ConversationSummary| std::cmp::Reverse(c.conversation.last_message_ts.unwrap_or(c.conversation.created_ms));
        if sort_alpha {
            out.others.sort_by_key(|c| title(c));
            out.archived.sort_by_key(|c| title(c));
        } else {
            out.others.sort_by_key(recent);
            out.archived.sort_by_key(recent);
        }
        out
    }

    /// Pins a conversation: it goes to the top of the pinned ones, on every device.
    pub async fn pin_conversation(&self, conv_id: &str) -> Result<()> {
        self.set_favourite(conv_id, true).await?;
        let mut order: Vec<String> = self.pinned_ids().into_iter().filter(|i| i != conv_id).collect();
        order.insert(0, conv_id.to_string());
        self.reorder_conversations(&order).await
    }

    /// Unpins a conversation; it joins the unpinned ones, sorted by the list setting.
    pub async fn unpin_conversation(&self, conv_id: &str) -> Result<()> {
        self.set_favourite(conv_id, false).await.map(|_| ())
    }

    /// Stores a new order for some pinned conversations (the ones one phone tab shows): they take the places the same
    /// number of pinned conversations had, so pinned conversations of the other tab keep theirs.
    pub async fn reorder_pinned(&self, subset: &[String]) -> Result<()> {
        let full = self.pinned_ids();
        let order = merge_pinned_order(&full, subset);
        if order == full {
            return Ok(());
        }
        self.reorder_conversations(&order).await
    }

    /// Drag and drop: moves the pinned conversation `src` before or after the pinned conversation `target` (same tab).
    pub async fn move_pinned_to(&self, src: &str, target: &str, after: bool) -> Result<bool> {
        let Some(kind) = self.conversations().into_iter().find(|c| c.conversation.id == src).map(|c| if c.conversation.kind == ConversationKind::Channel { ListKind::Channels } else { ListKind::Dms }) else { return Ok(false) };
        let mut mine: Vec<String> = self.phone_list(kind, false, false, false).pinned.into_iter().map(|c| c.conversation.id).collect();
        if !place_next_to(&mut mine, src, target, after) {
            return Ok(false);
        }
        self.reorder_pinned(&mine).await?;
        Ok(true)
    }

    /// Moves a pinned conversation up (`delta` -1) or down (+1) among the pinned ones of its own tab. False when it
    /// is not pinned or already at that end.
    pub async fn move_pinned(&self, conv_id: &str, delta: i32) -> Result<bool> {
        let Some(kind) = self.conversations().into_iter().find(|c| c.conversation.id == conv_id).map(|c| if c.conversation.kind == ConversationKind::Channel { ListKind::Channels } else { ListKind::Dms }) else { return Ok(false) };
        let mut mine: Vec<String> = self.phone_list(kind, false, false, false).pinned.into_iter().map(|c| c.conversation.id).collect();
        let Some(i) = mine.iter().position(|x| x == conv_id) else { return Ok(false) };
        let j = i as i32 + delta;
        if j < 0 || j as usize >= mine.len() {
            return Ok(false);
        }
        mine.swap(i, j as usize);
        self.reorder_pinned(&mine).await?;
        Ok(true)
    }
}

/// Puts `src` next to `target` (before it, or after it with `after`) in `order`. False when either is missing or equal.
pub fn place_next_to(order: &mut Vec<String>, src: &str, target: &str, after: bool) -> bool {
    if src == target || !order.iter().any(|i| i == src) {
        return false;
    }
    order.retain(|i| i != src);
    let Some(t) = order.iter().position(|i| i == target) else { return false };
    order.insert(if after { t + 1 } else { t }, src.to_string());
    true
}

/// `full` is the whole pinned order, `subset` a re-ordered selection of it. The selection's members are put back into
/// the slots they occupied, in the new order; everything else stays where it was.
pub fn merge_pinned_order(full: &[String], subset: &[String]) -> Vec<String> {
    let wanted: Vec<&String> = subset.iter().filter(|s| full.contains(s)).collect();
    let mut it = wanted.into_iter();
    full.iter().map(|id| if subset.contains(id) { it.next().cloned().unwrap_or_else(|| id.clone()) } else { id.clone() }).collect()
}

#[cfg(test)]
mod tests {
    use super::{merge_pinned_order, place_next_to};

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn subset_takes_its_own_slots() {
        // d1 c1 d2 c2 d3: the DMs are re-ordered, the channels stay where they are.
        let full = v(&["d1", "c1", "d2", "c2", "d3"]);
        assert_eq!(merge_pinned_order(&full, &v(&["d3", "d1", "d2"])), v(&["d3", "c1", "d1", "c2", "d2"]));
        assert_eq!(merge_pinned_order(&full, &v(&["d1", "d2", "d3"])), full);
    }

    #[test]
    fn dropping_next_to_a_target() {
        let mut o = v(&["a", "b", "c", "d"]);
        assert!(place_next_to(&mut o, "d", "a", false));
        assert_eq!(o, v(&["d", "a", "b", "c"]));
        assert!(place_next_to(&mut o, "d", "b", true));
        assert_eq!(o, v(&["a", "b", "d", "c"]));
        assert!(!place_next_to(&mut o, "a", "a", true));
        assert!(!place_next_to(&mut o, "x", "a", true));
        assert!(!place_next_to(&mut o, "a", "zzz", true));
    }

    #[test]
    fn unknown_ids_are_ignored() {
        let full = v(&["a", "b"]);
        assert_eq!(merge_pinned_order(&full, &v(&["x", "b", "a"])), v(&["b", "a"]));
    }
}

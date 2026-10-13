use qmsg_types::{Channel, ChannelRef, Message, ProviderStatus, User};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ProviderId {
    pub name: String,
    pub instance: u64,
}
#[derive(Clone, Default, Deserialize)]
pub struct Scope {
    #[serde(default)]
    pub users: BTreeMap<String, User>,
    #[serde(default)]
    pub channels: BTreeMap<String, Channel>,
}
#[derive(Clone, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub scope: Scope,
}
#[derive(Clone, Deserialize)]
pub struct Provider {
    pub provider: ProviderId,
    pub status: ProviderStatus,
    pub organizations: Vec<Organization>,
    pub standalone: Option<Scope>,
}
#[derive(Default, Deserialize)]
pub struct Snapshot {
    pub providers: Vec<Provider>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub provider: ProviderId,
    pub channel: ChannelRef,
}
#[derive(Clone)]
pub struct Room {
    pub target: Target,
    pub channel: Channel,
    pub group: String,
    pub users: BTreeMap<String, User>,
}
impl Provider {
    pub fn rooms(&self) -> Vec<Room> {
        let mut rooms = Vec::new();
        let scopes = self
            .standalone
            .iter()
            .map(|s| (None, "Direct messages", s))
            .chain(
                self.organizations
                    .iter()
                    .map(|o| (Some(o.id.clone()), o.name.as_str(), &o.scope)),
            );
        for (organization, group, scope) in scopes {
            let mut channels: Vec<_> = scope.channels.values().collect();
            channels.sort_by_key(|c| (c.position.unwrap_or(u32::MAX), c.name.clone()));
            for channel in channels {
                if matches!(channel.kind, qmsg_types::ChannelKind::Category) {
                    continue;
                }
                rooms.push(Room {
                    target: Target {
                        provider: self.provider.clone(),
                        channel: ChannelRef {
                            organization: organization.clone(),
                            channel: channel.id.clone(),
                        },
                    },
                    channel: channel.clone(),
                    group: group.into(),
                    users: scope.users.clone(),
                });
            }
        }
        rooms
    }
}
#[derive(Default)]
pub struct Timeline {
    pub messages: Vec<Message>,
    pub cursor: Option<String>,
}
impl Timeline {
    pub fn refresh(&mut self, page: qmsg_types::HistoryPage) {
        let Some(first) = page.messages.first() else {
            self.messages.clear();
            self.cursor = page.next_cursor;
            return;
        };
        if page.next_cursor.is_none() {
            self.messages = page.messages;
            self.cursor = None;
            return;
        }
        let start = self
            .messages
            .iter()
            .position(|m| m.id == first.id)
            .unwrap_or_else(|| {
                self.messages.partition_point(|m| {
                    (m.sent_at, m.id.as_str()) < (first.sent_at, first.id.as_str())
                })
            });
        self.messages.truncate(start);
        self.messages.extend(page.messages);
        // A recent page's cursor skips the older messages we kept above.
        if start == 0 {
            self.cursor = page.next_cursor;
        }
    }

    pub fn prepend(&mut self, page: qmsg_types::HistoryPage) {
        let mut older = page.messages;
        older.retain(|m| !self.messages.iter().any(|current| current.id == m.id));
        older.append(&mut self.messages);
        self.messages = older;
        self.cursor = page.next_cursor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refresh_replaces_edited_messages_and_removes_deletions_in_recent_page() {
        let channel = ChannelRef::new(None, "test");
        let message = |id: &str, time, body: &str| {
            Message::new(
                id,
                channel.clone(),
                "me",
                time,
                vec![qmsg_types::Content::Text(body.into())],
            )
        };
        let mut timeline = Timeline {
            messages: vec![
                message("1", 1, "older"),
                message("2", 2, "before edit"),
                message("3", 3, "deleted"),
            ],
            cursor: Some("older".into()),
        };
        timeline.refresh(qmsg_types::HistoryPage {
            messages: vec![message("2", 2, "edited"), message("4", 4, "new")],
            next_cursor: Some("recent".into()),
        });
        assert_eq!(
            timeline
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "4"]
        );
        assert_eq!(
            timeline.messages[1].content,
            vec![qmsg_types::Content::Text("edited".into())]
        );
        assert_eq!(timeline.cursor.as_deref(), Some("older"));
        timeline.refresh(qmsg_types::HistoryPage {
            messages: vec![],
            next_cursor: None,
        });
        assert!(timeline.messages.is_empty());
        assert!(timeline.cursor.is_none());
    }

    #[test]
    fn overlapping_older_pages_keep_current_message_version() {
        let channel = ChannelRef::new(None, "test");
        let message = |id: &str, body: &str| {
            Message::new(
                id,
                channel.clone(),
                "me",
                0,
                vec![qmsg_types::Content::Text(body.into())],
            )
        };
        let mut timeline = Timeline {
            messages: vec![message("2", "edited"), message("3", "new")],
            cursor: None,
        };
        timeline.prepend(qmsg_types::HistoryPage {
            messages: vec![message("1", "old"), message("2", "before edit")],
            next_cursor: Some("cursor".into()),
        });
        assert_eq!(
            timeline
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "3"]
        );
        assert_eq!(
            timeline.messages[1].content,
            vec![qmsg_types::Content::Text("edited".into())]
        );
    }
}

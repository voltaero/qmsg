use qmsg_types::{Channel, ChannelKind, ChannelRef, Message, ProviderStatus, User};
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
impl Target {
    /// Identifies the channel across provider restarts, which change `instance`.
    pub fn stable(&self) -> (String, ChannelRef) {
        (self.provider.name.clone(), self.channel.clone())
    }
}
/// A channel borrowed from its provider, so listing rooms copies no users.
pub struct Room<'a> {
    pub target: Target,
    pub channel: &'a Channel,
    pub users: &'a BTreeMap<String, User>,
}
/// A server or the direct messages of one account, shown in the server rail.
pub struct Group<'a> {
    pub organization: Option<&'a str>,
    pub name: &'a str,
    pub scope: &'a Scope,
}
/// A line of the channel list: a category heading or an openable channel.
pub struct Row<'a> {
    pub channel: &'a Channel,
    pub depth: usize,
    /// Index into [`Group::rooms`], or `None` for a category heading.
    pub room: Option<usize>,
}
impl Provider {
    /// Direct messages first, then servers in snapshot order.
    pub fn groups(&self) -> Vec<Group<'_>> {
        self.standalone
            .iter()
            .map(|scope| Group {
                organization: None,
                name: "Direct messages",
                scope,
            })
            .chain(self.organizations.iter().map(|o| Group {
                organization: Some(&o.id),
                name: &o.name,
                scope: &o.scope,
            }))
            .collect()
    }
    pub fn rooms(&self, group: usize) -> Vec<Room<'_>> {
        self.groups()
            .get(group)
            .map(|g| g.rooms(&self.provider))
            .unwrap_or_default()
    }
}
impl<'a> Group<'a> {
    /// Channels in tree order: each parent, then its children, by position.
    pub fn rows(&self) -> Vec<Row<'a>> {
        let channels = &self.scope.channels;
        let mut children: BTreeMap<Option<&str>, Vec<&Channel>> = BTreeMap::new();
        for channel in channels.values() {
            let parent = channel
                .parent
                .as_deref()
                .filter(|p| channels.contains_key(*p) && *p != channel.id);
            children.entry(parent).or_default().push(channel);
        }
        for list in children.values_mut() {
            // Uncategorized channels come before categories, as on Discord.
            list.sort_by_key(|c| {
                (
                    matches!(c.kind, ChannelKind::Category),
                    c.position.unwrap_or(u32::MAX),
                    c.name.clone(),
                )
            });
        }
        let mut rows = Vec::new();
        let mut stack: Vec<(&Channel, usize)> = children
            .get(&None)
            .map(|l| l.iter().rev().map(|c| (*c, 0)).collect())
            .unwrap_or_default();
        let mut seen = std::collections::BTreeSet::new();
        let mut rooms = 0;
        while let Some((channel, depth)) = stack.pop() {
            if !seen.insert(channel.id.as_str()) {
                continue;
            }
            let heading = matches!(channel.kind, ChannelKind::Category);
            rows.push(Row {
                channel,
                depth,
                room: (!heading).then_some(rooms),
            });
            rooms += usize::from(!heading);
            if let Some(list) = children.get(&Some(channel.id.as_str())) {
                stack.extend(list.iter().rev().map(|c| (*c, depth + 1)));
            }
        }
        rows
    }
    pub fn rooms(&self, provider: &ProviderId) -> Vec<Room<'a>> {
        self.rows()
            .into_iter()
            .filter(|row| row.room.is_some())
            .map(|row| Room {
                target: Target {
                    provider: provider.clone(),
                    channel: ChannelRef {
                        organization: self.organization.map(str::to_owned),
                        channel: row.channel.id.clone(),
                    },
                },
                channel: row.channel,
                users: &self.scope.users,
            })
            .collect()
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
    #[test]
    fn channel_rows_put_children_under_their_category_or_parent() {
        use qmsg_types::ChannelKind::{Category, Text, Thread};
        let mut scope = Scope::default();
        for (id, kind, parent, position) in [
            ("talk", Category, None, 1),
            ("lobby", Text, None, 5),
            ("general", Text, Some("talk"), 2),
            ("art", Text, Some("talk"), 1),
            ("idea", Thread, Some("general"), 0),
            ("lost", Text, Some("missing"), 0),
        ] {
            let mut channel = Channel::new(id, id, kind);
            channel.parent = parent.map(Into::into);
            channel.position = Some(position);
            scope.channels.insert(id.into(), channel);
        }
        let group = Group {
            organization: Some("org"),
            name: "Org",
            scope: &scope,
        };
        let rows: Vec<_> = group
            .rows()
            .iter()
            .map(|r| (r.channel.id.as_str(), r.depth, r.room))
            .collect();
        assert_eq!(
            rows,
            [
                ("lost", 0, Some(0)),
                ("lobby", 0, Some(1)),
                ("talk", 0, None),
                ("art", 1, Some(2)),
                ("general", 1, Some(3)),
                ("idea", 2, Some(4)),
            ]
        );
        let provider = ProviderId {
            name: "p".into(),
            instance: 1,
        };
        let rooms = group.rooms(&provider);
        assert_eq!(rooms.len(), 5);
        assert_eq!(
            rooms[3].target.channel,
            ChannelRef::new(Some("org"), "general")
        );
    }
}

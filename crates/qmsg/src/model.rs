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
    pub fn prepend(&mut self, page: qmsg_types::HistoryPage) {
        let mut older = page.messages;
        older.retain(|m| !self.messages.iter().any(|current| current.id == m.id));
        older.append(&mut self.messages);
        self.messages = older;
        self.cursor = page.next_cursor;
    }
}

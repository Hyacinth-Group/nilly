//! Registry ownership is synchronous: callers clone shared adapter handles before
//! awaiting. Durable room identity never includes the session's credential name.
use super::{
    interface::{Adapter, Platform, PlatformError, PlatformResult},
    outgoing::{ConnectionSnapshot, ConnectionStatus, RoomSnapshot},
    types::*,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
enum AdminPolicy {
    LegacyDiscordOpen,
    Users(HashSet<UserKey>),
}
impl AdminPolicy {
    fn allows(&self, user: &UserKey) -> bool {
        match self {
            Self::LegacyDiscordOpen => user.platform() == PlatformKind::Discord,
            Self::Users(users) => users.contains(user),
        }
    }
}
struct ConnectionEntry {
    adapter: Arc<Adapter>,
    status: ConnectionStatus,
    rooms: HashMap<ConversationKey, RoomRef>,
    administration: AdminPolicy,
}
#[derive(Default)]
pub struct ConnectionRegistry {
    connections: HashMap<ConnectionId, ConnectionEntry>,
}
impl ConnectionRegistry {
    pub fn register(&mut self, adapter: Adapter) -> Arc<Adapter> {
        let id = adapter.connection_id().clone();
        let adapter = Arc::new(adapter);
        let previous = self.connections.remove(&id);
        let (rooms, administration) = match previous {
            Some(entry) => (entry.rooms, entry.administration),
            None => (
                HashMap::new(),
                if adapter.self_user().id.platform() == PlatformKind::Discord {
                    AdminPolicy::LegacyDiscordOpen
                } else {
                    AdminPolicy::Users(HashSet::new())
                },
            ),
        };
        self.connections.insert(
            id,
            ConnectionEntry {
                adapter: adapter.clone(),
                status: ConnectionStatus::Ready,
                rooms,
                administration,
            },
        );
        adapter
    }
    pub fn get(&self, id: &ConnectionId) -> Option<Arc<Adapter>> {
        self.connections
            .get(id)
            .filter(|entry| entry.status == ConnectionStatus::Ready)
            .map(|entry| entry.adapter.clone())
    }
    pub fn connection_platform(&self, connection: &ConnectionId) -> Option<PlatformKind> {
        self.connections
            .get(connection)
            .filter(|entry| entry.status == ConnectionStatus::Ready)
            .map(|entry| entry.adapter.self_user().id.platform())
    }
    pub fn is_bound(&self, connection: &ConnectionId, room: &RoomRef) -> bool {
        self.connections.get(connection).is_some_and(|entry| {
            entry.status == ConnectionStatus::Ready
                && entry.rooms.contains_key(&room.conversation())
        })
    }
    pub fn bind(&mut self, connection: &ConnectionId, room: RoomRef) -> PlatformResult<()> {
        let entry = self
            .connections
            .get(connection)
            .ok_or(PlatformError::Disconnected)?;
        if entry.adapter.self_user().id.platform() != room.id.platform() {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        let conversation = room.conversation();
        if self
            .connections
            .iter()
            .any(|(id, entry)| id != connection && entry.rooms.contains_key(&conversation))
        {
            return Err(PlatformError::Transport(
                "conversation already bound to another connection".into(),
            ));
        }
        self.connections
            .get_mut(connection)
            .expect("connection checked")
            .rooms
            .insert(conversation, room);
        Ok(())
    }
    pub fn for_conversation(&self, conversation: &ConversationKey) -> Option<Arc<Adapter>> {
        self.connections
            .values()
            .find(|entry| {
                entry.status == ConnectionStatus::Ready && entry.rooms.contains_key(conversation)
            })
            .map(|entry| entry.adapter.clone())
    }
    pub fn for_room(&self, room: &RoomKey) -> Option<Arc<Adapter>> {
        let route = ConversationKey {
            room: room.clone(),
            thread: None,
        };
        self.for_conversation(&route).or_else(|| {
            // Temporary Discord-only routing compatibility during extraction.
            if room.platform() != PlatformKind::Discord {
                return None;
            }
            self.connections
                .get("discord-default")
                .filter(|entry| entry.status == ConnectionStatus::Ready)
                .map(|entry| entry.adapter.clone())
        })
    }
    pub fn room(&self, conversation: &ConversationKey) -> Option<RoomRef> {
        self.connections
            .values()
            .find_map(|entry| entry.rooms.get(conversation).cloned())
    }
    pub fn set_status(
        &mut self,
        connection: &ConnectionId,
        status: ConnectionStatus,
    ) -> PlatformResult<()> {
        self.connections
            .get_mut(connection)
            .ok_or(PlatformError::NotFound)?
            .status = status;
        Ok(())
    }
    /// New platforms require explicit, source-qualified administrator identities.
    pub fn set_admin_users(
        &mut self,
        connection: &ConnectionId,
        users: HashSet<UserKey>,
    ) -> PlatformResult<()> {
        self.connections
            .get_mut(connection)
            .ok_or(PlatformError::NotFound)?
            .administration = AdminPolicy::Users(users);
        Ok(())
    }
    pub fn authorize(&self, connection: &ConnectionId, user: &UserKey) -> PlatformResult<()> {
        let entry = self
            .connections
            .get(connection)
            .ok_or(PlatformError::Disconnected)?;
        if entry.status != ConnectionStatus::Ready {
            return Err(PlatformError::Disconnected);
        }
        if user.platform() != entry.adapter.self_user().id.platform() {
            return Err(PlatformError::PermissionDenied);
        }
        if entry.administration.allows(user) {
            Ok(())
        } else {
            Err(PlatformError::PermissionDenied)
        }
    }
    pub fn snapshot(&self) -> Vec<ConnectionSnapshot> {
        let mut connections: Vec<_> = self
            .connections
            .iter()
            .map(|(id, entry)| {
                let mut rooms: Vec<_> = entry
                    .rooms
                    .values()
                    .map(|room| RoomSnapshot {
                        room: room.clone(),
                        capabilities: entry.adapter.capabilities(room),
                    })
                    .collect();
                rooms.sort_by(|left, right| {
                    left.room
                        .id
                        .cmp(&right.room.id)
                        .then_with(|| left.room.thread.cmp(&right.room.thread))
                });
                ConnectionSnapshot {
                    connection: id.clone(),
                    platform: entry.adapter.self_user().id.platform(),
                    status: entry.status,
                    account: Some(entry.adapter.self_user().clone()),
                    rooms,
                }
            })
            .collect();
        connections.sort_by(|left, right| left.connection.cmp(&right.connection));
        connections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn administration_requires_explicit_qualified_identity_outside_legacy_discord() {
        let discord = UserKey::discord(42);
        let telegram = UserKey::new(PlatformKind::Telegram, &["42"]).unwrap();
        assert!(AdminPolicy::LegacyDiscordOpen.allows(&discord));
        assert!(!AdminPolicy::LegacyDiscordOpen.allows(&telegram));
        assert!(!AdminPolicy::Users(HashSet::new()).allows(&telegram));
        let policy = AdminPolicy::Users(HashSet::from([discord.clone()]));
        assert!(policy.allows(&discord));
        assert!(!policy.allows(&telegram));
    }
}

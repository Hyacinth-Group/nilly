//! Registry ownership is synchronous: snapshot shared handles before any await.
//! Discovery does not grant permission; new platforms require explicit room/admin policy.
use super::{
    capabilities::Capabilities,
    interface::{Adapter, Platform, PlatformError, PlatformResult},
    outgoing::{ConnectionSnapshot, ConnectionStatus, RoomSnapshot},
    types::*,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Debug, Clone)]
pub enum RoomPolicy {
    LegacyDiscord {
        space: Option<SpaceKey>,
        rooms: Option<HashSet<RoomKey>>,
    },
    Approved(HashSet<RoomKey>),
}
impl RoomPolicy {
    fn allows(&self, room: &RoomRef) -> bool {
        match self {
            Self::LegacyDiscord { space, rooms } => {
                room.id.platform() == PlatformKind::Discord
                    && space
                        .as_ref()
                        .is_none_or(|space| room.space.as_ref() == Some(space))
                    && rooms.as_ref().is_none_or(|rooms| rooms.contains(&room.id))
            }
            Self::Approved(rooms) => rooms.contains(&room.id),
        }
    }
}
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
    platform: PlatformKind,
    adapter: Option<Arc<Adapter>>,
    status: ConnectionStatus,
    rooms: HashMap<ConversationKey, RoomRef>,
    spaces: Vec<SpaceRef>,
    routing: RoomPolicy,
    administration: AdminPolicy,
}
impl ConnectionEntry {
    fn configured(platform: PlatformKind) -> Self {
        Self {
            platform,
            adapter: None,
            status: ConnectionStatus::Connecting,
            rooms: HashMap::new(),
            spaces: Vec::new(),
            routing: if platform == PlatformKind::Discord {
                RoomPolicy::LegacyDiscord {
                    space: None,
                    rooms: None,
                }
            } else {
                RoomPolicy::Approved(HashSet::new())
            },
            administration: if platform == PlatformKind::Discord {
                AdminPolicy::LegacyDiscordOpen
            } else {
                AdminPolicy::Users(HashSet::new())
            },
        }
    }
}
#[derive(Default)]
pub struct ConnectionRegistry {
    connections: HashMap<ConnectionId, ConnectionEntry>,
}
impl ConnectionRegistry {
    pub fn configure(&mut self, id: &ConnectionId, platform: PlatformKind) -> PlatformResult<()> {
        if let Some(existing) = self.connections.get(id) {
            return if existing.platform == platform {
                Ok(())
            } else {
                Err(PlatformError::InvalidIdentity(KeyError))
            };
        }
        self.connections
            .insert(id.clone(), ConnectionEntry::configured(platform));
        Ok(())
    }
    pub fn register(&mut self, adapter: Adapter) -> Arc<Adapter> {
        let id = adapter.connection_id().clone();
        let platform = adapter.self_user().id.platform();
        let adapter = Arc::new(adapter);
        let entry = self
            .connections
            .entry(id)
            .or_insert_with(|| ConnectionEntry::configured(platform));
        entry.platform = platform;
        entry.status = ConnectionStatus::Ready;
        entry.adapter = Some(adapter.clone());
        adapter
    }
    pub fn get(&self, id: &ConnectionId) -> Option<Arc<Adapter>> {
        self.connections
            .get(id)
            .filter(|entry| entry.status == ConnectionStatus::Ready)
            .and_then(|entry| entry.adapter.clone())
    }
    pub fn ready_adapters(&self) -> Vec<Arc<Adapter>> {
        self.connections
            .values()
            .filter(|entry| entry.status == ConnectionStatus::Ready)
            .filter_map(|entry| entry.adapter.clone())
            .collect()
    }
    pub fn connection_platform(&self, id: &ConnectionId) -> Option<PlatformKind> {
        self.connections
            .get(id)
            .filter(|entry| entry.status == ConnectionStatus::Ready)
            .map(|entry| entry.platform)
    }
    pub fn configured_platform(&self, id: &ConnectionId) -> Option<PlatformKind> {
        self.connections.get(id).map(|entry| entry.platform)
    }
    pub fn set_room_policy(&mut self, id: &ConnectionId, policy: RoomPolicy) -> PlatformResult<()> {
        self.connections
            .get_mut(id)
            .ok_or(PlatformError::NotFound)?
            .routing = policy;
        Ok(())
    }
    pub fn allows(&self, id: &ConnectionId, room: &RoomRef) -> bool {
        self.connections
            .get(id)
            .is_some_and(|entry| entry.platform == room.id.platform() && entry.routing.allows(room))
    }
    /// Explicit trusted-operator or legacy native-command binding. Ordinary discovery
    /// uses bind_discovered instead and cannot bypass the configured room policy.
    pub fn bind(&mut self, id: &ConnectionId, room: RoomRef) -> PlatformResult<()> {
        let entry = self
            .connections
            .get(id)
            .ok_or(PlatformError::Disconnected)?;
        if entry.adapter.is_none() || entry.status != ConnectionStatus::Ready {
            return Err(PlatformError::Disconnected);
        }
        if entry.platform != room.id.platform()
            || room
                .space
                .as_ref()
                .is_some_and(|space| space.platform() != entry.platform)
            || room
                .thread
                .as_ref()
                .is_some_and(|thread| thread.platform() != entry.platform)
        {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        if self.connections.iter().any(|(other, entry)| {
            other != id
                && entry
                    .rooms
                    .keys()
                    .any(|conversation| conversation.room == room.id)
        }) {
            return Err(PlatformError::Transport(
                "room already bound to another connection".into(),
            ));
        }
        self.connections
            .get_mut(id)
            .expect("connection checked")
            .rooms
            .insert(room.conversation(), room);
        Ok(())
    }
    pub fn bind_discovered(&mut self, id: &ConnectionId, room: RoomRef) -> PlatformResult<bool> {
        if !self.allows(id, &room) {
            return Ok(false);
        }
        self.bind(id, room)?;
        Ok(true)
    }
    pub fn is_bound(&self, id: &ConnectionId, room: &RoomRef) -> bool {
        self.connections.get(id).is_some_and(|entry| {
            entry.status == ConnectionStatus::Ready
                && entry.rooms.contains_key(&room.conversation())
        })
    }
    pub fn for_conversation(&self, conversation: &ConversationKey) -> Option<Arc<Adapter>> {
        self.connections
            .values()
            .find(|entry| {
                entry.status == ConnectionStatus::Ready && entry.rooms.contains_key(conversation)
            })
            .and_then(|entry| entry.adapter.clone())
    }
    pub fn room(&self, conversation: &ConversationKey) -> Option<RoomRef> {
        self.connections
            .values()
            .find(|entry| entry.status == ConnectionStatus::Ready)
            .and_then(|entry| entry.rooms.get(conversation).cloned())
            .or_else(|| {
                self.connections
                    .values()
                    .filter(|entry| entry.status == ConnectionStatus::Ready)
                    .find_map(|entry| entry.rooms.get(conversation).cloned())
            })
    }
    pub fn record_space(&mut self, id: &ConnectionId, space: SpaceRef) -> PlatformResult<()> {
        let entry = self
            .connections
            .get_mut(id)
            .ok_or(PlatformError::Disconnected)?;
        if entry.platform != space.id.platform() {
            return Err(PlatformError::InvalidIdentity(KeyError));
        }
        if let Some(previous) = entry
            .spaces
            .iter_mut()
            .find(|previous| previous.id == space.id)
        {
            *previous = space;
        } else {
            entry.spaces.push(space);
        }
        Ok(())
    }
    pub fn discovered_spaces(&self) -> Vec<(ConnectionId, SpaceRef)> {
        self.connections
            .iter()
            .filter(|(_, entry)| entry.status == ConnectionStatus::Ready)
            .flat_map(|(id, entry)| {
                entry
                    .spaces
                    .iter()
                    .map(move |space| (id.clone(), space.clone()))
            })
            .collect()
    }
    pub fn set_status(
        &mut self,
        id: &ConnectionId,
        status: ConnectionStatus,
    ) -> PlatformResult<()> {
        self.connections
            .get_mut(id)
            .ok_or(PlatformError::NotFound)?
            .status = status;
        Ok(())
    }
    pub fn set_admin_users(
        &mut self,
        id: &ConnectionId,
        users: HashSet<UserKey>,
    ) -> PlatformResult<()> {
        self.connections
            .get_mut(id)
            .ok_or(PlatformError::NotFound)?
            .administration = AdminPolicy::Users(users);
        Ok(())
    }
    pub fn authorize(&self, id: &ConnectionId, user: &UserKey) -> PlatformResult<()> {
        let entry = self
            .connections
            .get(id)
            .ok_or(PlatformError::Disconnected)?;
        if entry.status != ConnectionStatus::Ready || entry.adapter.is_none() {
            return Err(PlatformError::Disconnected);
        }
        if entry.platform != user.platform() || !entry.administration.allows(user) {
            return Err(PlatformError::PermissionDenied);
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Vec<ConnectionSnapshot> {
        let mut snapshots: Vec<_> = self
            .connections
            .iter()
            .map(|(id, entry)| {
                let mut rooms: Vec<_> = entry
                    .rooms
                    .values()
                    .map(|room| RoomSnapshot {
                        room: room.clone(),
                        capabilities: if entry.status == ConnectionStatus::Ready {
                            entry
                                .adapter
                                .as_ref()
                                .map(|adapter| adapter.capabilities(room))
                                .unwrap_or_else(Capabilities::unavailable)
                        } else {
                            Capabilities::unavailable()
                        },
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
                    platform: entry.platform,
                    status: entry.status,
                    account: entry
                        .adapter
                        .as_ref()
                        .map(|adapter| adapter.self_user().clone()),
                    rooms,
                    warnings: entry
                        .adapter
                        .as_ref()
                        .map(|adapter| adapter.connection_warnings())
                        .unwrap_or_default(),
                }
            })
            .collect();
        snapshots.sort_by(|left, right| left.connection.cmp(&right.connection));
        snapshots
    }
}

impl ConnectionRegistry {
    pub fn first_approved_room(&self) -> Option<ConversationKey> {
        self.connections
            .iter()
            .filter(|(_, entry)| entry.status == ConnectionStatus::Ready)
            .flat_map(|(connection, entry)| {
                entry
                    .rooms
                    .iter()
                    .filter(|(_, room)| entry.routing.allows(room))
                    .map(move |(key, _)| (connection, key))
            })
            .min_by(|(left_id, left), (right_id, right)| {
                left_id
                    .cmp(right_id)
                    .then_with(|| left.room.cmp(&right.room))
                    .then_with(|| left.thread.cmp(&right.thread))
            })
            .map(|(_, key)| key.clone())
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
    #[test]
    fn configured_connections_are_visible_without_fake_connected_adapters() {
        let mut registry = ConnectionRegistry::default();
        let id = ConnectionId::new("telegram-family");
        registry.configure(&id, PlatformKind::Telegram).unwrap();
        assert!(registry.get(&id).is_none());
        assert_eq!(registry.snapshot()[0].status, ConnectionStatus::Connecting);
        assert!(registry.snapshot()[0].account.is_none());
        assert!(registry.configure(&id, PlatformKind::Discord).is_err());
        registry
            .set_status(&id, ConnectionStatus::Disconnected)
            .unwrap();
        assert_eq!(
            registry.snapshot()[0].status,
            ConnectionStatus::Disconnected
        );
    }
    #[test]
    fn approval_separates_native_rooms_and_threads_from_identity_names() {
        let id = ConnectionId::new("telegram-family");
        let room = RoomKey::new(PlatformKind::Telegram, &["-100123"]).unwrap();
        let mut metadata = RoomRef::from_key(room.clone());
        metadata.thread = Some(ThreadKey::new(PlatformKind::Telegram, &["-100123", "81"]).unwrap());
        let mut registry = ConnectionRegistry::default();
        registry.configure(&id, PlatformKind::Telegram).unwrap();
        assert!(!registry.allows(&id, &metadata));
        registry
            .set_room_policy(&id, RoomPolicy::Approved(HashSet::from([room])))
            .unwrap();
        assert!(registry.allows(&id, &metadata));
        assert!(registry.bind_discovered(&id, metadata).is_err());
    }
}

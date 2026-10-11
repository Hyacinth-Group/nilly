//! Registry ownership is synchronous: callers clone shared adapter handles before
//! awaiting. Durable room identity never includes the session's credential name.
use super::{
    interface::{Adapter, Platform, PlatformError, PlatformResult},
    outgoing::{ConnectionSnapshot, ConnectionStatus, RoomSnapshot},
    types::*,
};
use std::{collections::HashMap, sync::Arc};
struct ConnectionEntry {
    adapter: Arc<Adapter>,
    status: ConnectionStatus,
    rooms: HashMap<ConversationKey, RoomRef>,
}
#[derive(Default)]
pub struct ConnectionRegistry {
    connections: HashMap<ConnectionId, ConnectionEntry>,
}
impl ConnectionRegistry {
    pub fn register(&mut self, adapter: Adapter) -> Arc<Adapter> {
        let id = adapter.connection_id().clone();
        let adapter = Arc::new(adapter);
        let rooms = self
            .connections
            .remove(&id)
            .map(|entry| entry.rooms)
            .unwrap_or_default();
        self.connections.insert(
            id,
            ConnectionEntry {
                adapter: adapter.clone(),
                status: ConnectionStatus::Ready,
                rooms,
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

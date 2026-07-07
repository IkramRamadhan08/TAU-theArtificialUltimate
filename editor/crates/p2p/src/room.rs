use crate::protocol::{PeerId, PeerInfo, RoomId};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Participant {
    pub peer_id: PeerId,
    pub name: String,
    pub project_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Room {
    pub room_id: RoomId,
    pub name: String,
    pub participants: HashMap<PeerId, Participant>,
}

impl Room {
    pub fn new(room_id: RoomId, name: String) -> Self {
        Self {
            room_id,
            name,
            participants: HashMap::new(),
        }
    }

    pub fn add_participant(&mut self, info: PeerInfo) {
        self.participants.entry(info.peer_id).or_insert(Participant {
            peer_id: info.peer_id,
            name: info.name,
            project_id: None,
        });
    }

    pub fn remove_participant(&mut self, peer_id: PeerId) {
        self.participants.remove(&peer_id);
    }

    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.participants.is_empty()
    }
}

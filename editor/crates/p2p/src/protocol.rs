use serde::{Deserialize, Serialize};

pub type RoomId = u64;
pub type PeerId = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Register { name: String },
    CreateRoom { name: String },
    JoinRoom { room_id: RoomId },
    LeaveRoom,
    Relay { to: PeerId, payload: Vec<u8> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome { peer_id: PeerId, peers: Vec<PeerInfo> },
    PeerJoined(PeerInfo),
    PeerLeft { peer_id: PeerId },
    RoomCreated { room_id: RoomId },
    RoomJoined { room_id: RoomId, participants: Vec<PeerInfo> },
    ParticipantJoined { room_id: RoomId, peer: PeerInfo },
    ParticipantLeft { room_id: RoomId, peer_id: PeerId },
    Relay { from: PeerId, payload: Vec<u8> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerMessage {
    ShareProject { project_id: u64, name: String },
    UnshareProject { project_id: u64 },
    JoinProject { project_id: u64 },
    SyncStep1 { buffer_id: u64, data: Vec<u8> },
    SyncStep2 { buffer_id: u64, data: Vec<u8> },
    Update { buffer_id: u64, data: Vec<u8> },
    Follow { leader: PeerId },
    Unfollow { leader: PeerId },
    UpdateFollowers { leaders: Vec<(PeerId, Vec<PeerId>)> },
    ParticipantLocationChanged { location: u64 },
}

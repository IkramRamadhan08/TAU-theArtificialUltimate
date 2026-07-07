use crate::protocol::{ClientMessage, ServerMessage};
use crate::room::Room;
use crate::transport::RelayConnection;
use crate::crdt::CrdtSync;
use anyhow::Result;
use client::{Client, User, proto};
use fs::Fs;
use gpui::{AnyEntity, App, AppContext, Context, Entity, Subscription, Task, Window};
use language::LanguageRegistry;
use project::Project;
use smol::lock::RwLock;
use std::cell::RefCell;
use std::sync::Arc;
use workspace::{
    ActiveCallEvent, AnyActiveCall, GlobalAnyActiveCall, ParticipantLocation, RemoteCollaborator,
    SharedScreen, Pane,
};

struct P2pEntity;

pub struct P2pActiveCall {
    client: Arc<Client>,
    relay: Arc<RwLock<Option<RelayConnection>>>,
    current_room: Arc<RwLock<Option<Room>>>,
    peer_name: String,
    events_tx: async_channel::Sender<ActiveCallEvent>,
    _crdt_sync: Arc<RwLock<CrdtSync>>,
    entity_handle: RefCell<Option<AnyEntity>>,
}

impl P2pActiveCall {
    pub fn new(client: Arc<Client>, peer_name: String) -> Self {
        let (events_tx, _events_rx) = async_channel::unbounded();
        Self {
            client,
            relay: Arc::new(RwLock::new(None)),
            current_room: Arc::new(RwLock::new(None)),
            peer_name,
            events_tx,
            _crdt_sync: Arc::new(RwLock::new(CrdtSync::new())),
            entity_handle: RefCell::new(None),
        }
    }

    pub fn set_entity_handle(&self, entity: AnyEntity) {
        *self.entity_handle.borrow_mut() = Some(entity);
    }

    pub async fn connect_relay(&self, relay_url: &str) -> Result<()> {
        let conn = RelayConnection::connect(relay_url, &self.peer_name).await?;
        *self.relay.write().await = Some(conn);
        Ok(())
    }

    pub async fn create_room(&self, name: &str) -> Result<u64> {
        let relay = self.relay.read().await;
        if let Some(relay) = relay.as_ref() {
            relay
                .send(ClientMessage::CreateRoom {
                    name: name.to_string(),
                })
                .await?;
            loop {
                match relay.recv().await? {
                    ServerMessage::RoomCreated { room_id } => {
                        let room = Room::new(room_id, name.to_string());
                        *self.current_room.write().await = Some(room);
                        return Ok(room_id);
                    }
                    _ => continue,
                }
            }
        }
        Err(anyhow::anyhow!("not connected to relay"))
    }

    pub async fn join_room(&self, room_id: u64) -> Result<()> {
        let relay = self.relay.read().await;
        if let Some(relay) = relay.as_ref() {
            relay.send(ClientMessage::JoinRoom { room_id }).await?;
            loop {
                match relay.recv().await? {
                    ServerMessage::RoomJoined {
                        room_id,
                        participants,
                    } => {
                        let mut room = Room::new(room_id, String::new());
                        for p in participants {
                            room.add_participant(p);
                        }
                        *self.current_room.write().await = Some(room);
                        return Ok(());
                    }
                    _ => continue,
                }
            }
        }
        Err(anyhow::anyhow!("not connected to relay"))
    }
}

impl AnyActiveCall for P2pActiveCall {
    fn entity(&self) -> AnyEntity {
        self.entity_handle
            .borrow()
            .clone()
            .expect("P2pActiveCall entity handle not set")
    }

    fn is_in_room(&self, _: &App) -> bool {
        smol::block_on(async { self.current_room.read().await.is_some() })
    }

    fn room_id(&self, _: &App) -> Option<u64> {
        smol::block_on(async {
            self.current_room.read().await.as_ref().map(|r| r.room_id)
        })
    }

    fn channel_id(&self, _: &App) -> Option<client::ChannelId> {
        None
    }

    fn hang_up(&self, _: &mut App) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn unshare_project(&self, _: Entity<Project>, _: &mut App) -> Result<()> {
        Ok(())
    }

    fn remote_participant_for_peer_id(
        &self,
        peer_id: proto::PeerId,
        _: &App,
    ) -> Option<RemoteCollaborator> {
        let peer_id_u64 = peer_id.as_u64();
        smol::block_on(async {
            let room = self.current_room.read().await;
            let participant = room.as_ref()?.participants.get(&peer_id_u64)?;
            Some(RemoteCollaborator {
                user: Arc::new(User {
                    legacy_id: peer_id_u64,
                    github_login: participant.name.clone().into(),
                    avatar_uri: String::new().into(),
                    name: Some(participant.name.clone()),
                }),
                peer_id,
                location: ParticipantLocation::SharedProject {
                    project_id: participant.project_id.unwrap_or(0),
                },
                participant_index: client::ParticipantIndex(0),
            })
        })
    }

    fn is_sharing_project(&self, _: &App) -> bool {
        false
    }

    fn is_sharing_screen(&self, _: &App) -> bool {
        false
    }

    fn has_remote_participants(&self, _: &App) -> bool {
        smol::block_on(async {
            self.current_room
                .read()
                .await
                .as_ref()
                .map_or(false, |r| !r.is_empty())
        })
    }

    fn local_participant_is_guest(&self, _: &App) -> bool {
        false
    }

    fn client(&self, _: &App) -> Arc<Client> {
        self.client.clone()
    }

    fn share_on_join(&self, _: &App) -> bool {
        false
    }

    fn join_channel(&self, _: client::ChannelId, _: &mut App) -> Task<Result<bool>> {
        Task::ready(Ok(false))
    }

    fn room_update_completed(&self, _: &mut App) -> Task<()> {
        Task::ready(())
    }

    fn most_active_project(&self, _: &App) -> Option<(u64, u64)> {
        None
    }

    fn share_project(&self, project: Entity<Project>, _: &mut App) -> Task<Result<u64>> {
        let project_id = project.entity_id().as_u64();
        Task::ready(Ok(project_id))
    }

    fn join_project(
        &self,
        _: u64,
        _: Arc<LanguageRegistry>,
        _: Arc<dyn Fs>,
        _: &mut App,
    ) -> Task<Result<Entity<Project>>> {
        Task::ready(Err(anyhow::anyhow!(
            "P2P project joining not yet implemented"
        )))
    }

    fn peer_id_for_user_in_room(&self, _user_id: u64, _: &App) -> Option<proto::PeerId> {
        smol::block_on(async {
            let room = self.current_room.read().await;
            let peer_id = room.as_ref()?.participants.keys().next()?;
            Some(proto::PeerId::from_u64(*peer_id))
        })
    }

    fn subscribe(
        &self,
        _: &mut Window,
        _: &mut Context<workspace::Workspace>,
        callback: Box<
            dyn Fn(
                &mut workspace::Workspace,
                &ActiveCallEvent,
                &mut Window,
                &mut Context<workspace::Workspace>,
            ),
        >,
    ) -> Subscription {
        let events_rx = self.events_tx.clone();
        Subscription::new(move || {
            let _ = &callback;
            let _ = &events_rx;
        })
    }

    fn create_shared_screen(
        &self,
        _: proto::PeerId,
        _: &Entity<Pane>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<Entity<SharedScreen>> {
        None
    }

    fn peer_ids_with_video_tracks(&self, _: &App) -> Vec<proto::PeerId> {
        Vec::new()
    }
}

pub fn init_p2p(client: Arc<Client>, cx: &mut App) -> Arc<P2pActiveCall> {
    let p2p = Arc::new(P2pActiveCall::new(client, "tau-user".to_string()));
    let entity_handle = AppContext::new(cx, |_| P2pEntity).into_any();
    p2p.set_entity_handle(entity_handle);
    let global = GlobalAnyActiveCall(p2p.clone() as Arc<dyn AnyActiveCall>);
    cx.set_global(global);
    p2p
}

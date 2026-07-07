use crate::protocol::{ClientMessage, ServerMessage};
use anyhow::Result;
use async_tungstenite::tungstenite::{Message, Utf8Bytes};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct RelayConnection {
    peer_id: u64,
    write_tx: async_channel::Sender<ClientMessage>,
    read_rx: async_channel::Receiver<ServerMessage>,
    running: Arc<AtomicBool>,
}

impl RelayConnection {
    pub async fn connect(url: &str, name: &str) -> Result<Self> {
        let (ws, _) = async_tungstenite::tokio::connect_async_with_config(url, None).await?;
        let (mut write, mut read) = ws.split();

        let (client_tx, client_rx) = async_channel::unbounded::<ClientMessage>();
        let (server_tx, server_rx) = async_channel::unbounded::<ServerMessage>();
        let running = Arc::new(AtomicBool::new(true));

        let register = ClientMessage::Register {
            name: name.to_string(),
        };
        let msg = serde_json::to_string(&register)?;
        write.send(Message::Text(Utf8Bytes::from(msg))).await?;

        let mut peer_id = 0u64;

        while let Some(Ok(msg)) = read.next().await {
            if let Message::Text(text) = msg {
                let server_msg: ServerMessage = serde_json::from_str(&text)?;
                if let ServerMessage::Welcome { peer_id: id, .. } = &server_msg {
                    peer_id = *id;
                    server_tx.send(server_msg).await?;
                    break;
                }
            }
        }

        let running_clone = running.clone();
        smol::spawn(async move {
            loop {
                futures_util::future::select(
                    Box::pin(async {
                        if let Ok(msg) = client_rx.recv().await {
                            if let Ok(text) = serde_json::to_string(&msg) {
                                let _ = write.send(Message::Text(Utf8Bytes::from(text))).await;
                            }
                        }
                    }),
                    Box::pin(async {
                        if let Some(Ok(msg)) = read.next().await {
                            if let Message::Text(text) = msg {
                                if let Ok(server_msg) =
                                    serde_json::from_str::<ServerMessage>(&text)
                                {
                                    let _ = server_tx.send(server_msg).await;
                                }
                            }
                        }
                    }),
                )
                .await;
                if !running_clone.load(Ordering::Relaxed) {
                    break;
                }
            }
        })
        .detach();

        Ok(Self {
            peer_id,
            write_tx: client_tx,
            read_rx: server_rx,
            running,
        })
    }

    pub fn peer_id(&self) -> u64 {
        self.peer_id
    }

    pub async fn send(&self, msg: ClientMessage) -> Result<()> {
        self.write_tx.send(msg).await?;
        Ok(())
    }

    pub async fn recv(&self) -> Result<ServerMessage> {
        let msg = self.read_rx.recv().await?;
        Ok(msg)
    }

    #[allow(dead_code)]
    pub fn try_recv(&self) -> Option<ServerMessage> {
        self.read_rx.try_recv().ok()
    }
}

impl Drop for RelayConnection {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

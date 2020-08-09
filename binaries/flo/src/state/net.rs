use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing_futures::Instrument;

use flo_config::ClientConfig;
use flo_net::packet::FloPacket;

use crate::error::{Error, Result};
use crate::net::lobby::{LobbyStream, LobbyStreamSender};
pub use crate::net::node::NodesConfigSenderRef;
use crate::net::node::{NodeRegistry, NodeRegistryRef, PingUpdate};
use crate::ws::message::{Disconnect, DisconnectReason};
use crate::ws::{OutgoingMessage, WsSenderRef};

#[derive(Debug)]
pub struct NetState {
  lobby: LobbyState,
  nodes: NodeRegistryRef,
}

impl NetState {
  pub async fn init(config: &ClientConfig) -> Result<Self> {
    let (ping_sender, ping_receiver) = mpsc::channel(1);
    let nodes = NodeRegistry::new(ping_sender).into_ref();

    Ok(Self {
      lobby: LobbyState::new(&config.lobby_domain, ping_receiver),
      nodes,
    })
  }

  pub async fn connect_lobby(&self, ws_sender: WsSenderRef, token: String) -> Result<()> {
    Ok(
      self
        .lobby
        .connect(ws_sender, self.nodes.clone(), token)
        .await?,
    )
  }

  pub async fn lobby_send<T>(&self, packet: T) -> Result<()>
  where
    T: FloPacket,
  {
    let mut sender = self
      .lobby
      .get_sender_cloned()
      .ok_or_else(|| Error::ServerNotConnected)?;
    if let Err(_) = sender.send(packet.encode_as_frame()?).await {
      tracing::debug!("sender dropped");
      self.lobby.close().await;
      return Err(Error::ServerNotConnected);
    }
    Ok(())
  }
}

#[derive(Debug)]
struct LobbyState {
  domain: String,
  conn: Arc<RwLock<Option<LobbyConn>>>,
}

#[derive(Debug)]
struct LobbyConn {
  stream: LobbyStream,
  ws_sender: WsSenderRef,
}

impl LobbyState {
  pub fn new(domain: &str, mut ping_receiver: mpsc::Receiver<PingUpdate>) -> Self {
    let conn = Arc::new(RwLock::new(None::<LobbyConn>));

    tokio::spawn({
      let conn = conn.clone();
      async move {
        loop {
          while let Some(update) = ping_receiver.recv().await {
            let sender: Option<WsSenderRef> = { conn.read().as_ref().map(|c| c.ws_sender.clone()) };
            if let Some(sender) = sender {
              let r = sender.send(OutgoingMessage::PingUpdate(update)).await;
              if let Err(e) = r {
                tracing::debug!("send ping update: {}", e);
              }
            }
          }
          tracing::debug!("exiting");
        }
      }
      .instrument(tracing::debug_span!("ping_update_sender_worker"))
    });

    LobbyState {
      domain: domain.to_string(),
      conn,
    }
  }

  pub async fn connect(
    &self,
    ws_sender: WsSenderRef,
    nodes: NodeRegistryRef,
    token: String,
  ) -> Result<()> {
    let stream = LobbyStream::connect(&self.domain, ws_sender.clone(), nodes, token).await?;

    {
      *self.conn.write() = Some(LobbyConn { stream, ws_sender })
    }

    Ok(())
  }

  async fn close(&self) {
    let conn = self.conn.write().take();
    if let Some(conn) = conn {
      let r = conn
        .ws_sender
        .send(OutgoingMessage::Disconnect(Disconnect {
          reason: DisconnectReason::Unknown,
          message: "Server connection closed unexpectedly".to_string(),
        }))
        .await;
      if let Err(e) = r {
        tracing::debug!("send disconnect: {}", e);
      }
    }
  }

  fn get_sender_cloned(&self) -> Option<LobbyStreamSender> {
    self
      .conn
      .read()
      .as_ref()
      .map(|s| s.stream.get_sender_cloned())
  }
}
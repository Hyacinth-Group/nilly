//! Bounded ingress and independently supervised connections; SDK runners live in adapters.
use super::{types::*, PlatformError, PlatformResult};
use crate::{wait_for_shutdown, Bot, BotError, SHUTTING_DOWN};
use std::{
    collections::{HashSet, VecDeque},
    sync::{atomic::Ordering, Arc},
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
pub type ShutdownReceiver = watch::Receiver<bool>;
fn model_work(event: &ChatEvent) -> bool {
    matches!(
        event,
        ChatEvent::MessageCreated(_) | ChatEvent::ReactionAdded(_) | ChatEvent::MemberJoined { .. }
    )
}
/// Sending awaits bounded capacity, never a model call. Event IDs are source-qualified.
#[derive(Clone)]
pub struct EventSender {
    data: mpsc::Sender<EventEnvelope>,
    control: Option<mpsc::Sender<EventEnvelope>>,
}
impl EventSender {
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<EventEnvelope>) {
        let (data, receiver) = mpsc::channel(capacity);
        (
            Self {
                data,
                control: None,
            },
            receiver,
        )
    }
    pub async fn send(&self, event: EventEnvelope) -> PlatformResult<()> {
        let sender = if !model_work(&event.event) {
            self.control.as_ref().unwrap_or(&self.data)
        } else {
            &self.data
        };
        sender
            .send(event)
            .await
            .map_err(|_| PlatformError::Disconnected)
    }
}
/// Credentials stay adapter-local and must never be included in status snapshots.
pub enum ConnectionConfig {
    Discord {
        connection: ConnectionId,
        token: String,
    },
    #[cfg(feature = "matrix")]
    Matrix {
        connection: ConnectionId,
        config: super::matrix::MatrixConfig,
    },
}
struct SeenEvents {
    set: HashSet<(ConnectionId, EventKey)>,
    order: VecDeque<(ConnectionId, EventKey)>,
    capacity: usize,
}
impl SeenEvents {
    fn new(capacity: usize) -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
            capacity,
        }
    }
    fn accept(&mut self, connection: &ConnectionId, event: &EventKey) -> bool {
        let key = (connection.clone(), event.clone());
        if !self.set.insert(key.clone()) {
            return false;
        }
        self.order.push_back(key);
        if self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
        true
    }
}
/// Start all configured adapters once. Disconnecting one adapter leaves the rest alive.
pub(crate) async fn run(bot: Arc<Bot>, configs: Vec<ConnectionConfig>) -> Result<(), BotError> {
    let (mut ingress, mut receiver) = EventSender::channel(256);
    let (control, mut lifecycle) = mpsc::channel(64);
    ingress.control = Some(control);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut runners = JoinSet::new();
    let mut ids = HashSet::new();
    for config in &configs {
        let (connection, kind) = match config {
            ConnectionConfig::Discord { connection, .. } => (connection, PlatformKind::Discord),
            #[cfg(feature = "matrix")]
            ConnectionConfig::Matrix { connection, .. } => (connection, PlatformKind::Matrix),
        };
        if !ids.insert(connection.clone()) {
            return Err(PlatformError::Transport("duplicate configured connection".into()).into());
        }
        bot.adapters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .configure(connection, kind)?;
    }
    for config in configs {
        let connection = match &config {
            ConnectionConfig::Discord { connection, .. } => connection.clone(),
            #[cfg(feature = "matrix")]
            ConnectionConfig::Matrix { connection, .. } => connection.clone(),
        };
        let bot = bot.clone();
        let ingress = ingress.clone();
        let shutdown = shutdown_rx.clone();
        runners.spawn(async move {
            let result = match config {
                ConnectionConfig::Discord { token, .. } => {
                    super::discord::runner::run(bot, connection.clone(), token, ingress, shutdown)
                        .await
                }
                #[cfg(feature = "matrix")]
                ConnectionConfig::Matrix { config, .. } => {
                    super::matrix::runner::run(bot, connection.clone(), config, ingress, shutdown)
                        .await
                }
            };
            (connection, result)
        });
    }
    drop(ingress);
    let brain = bot.clone();
    let mut processor = tokio::spawn(async move {
        let mut seen = SeenEvents::new(4096);
        let mut jobs = JoinSet::new();
        let (mut data_open, mut control_open) = (true, true);
        loop {
            tokio::select! {
                event=lifecycle.recv(),if control_open=>match event {Some(event)=>{if seen.accept(&event.connection,&event.event_id){brain.handle_event(event).await;}},None=>control_open=false},
                event=receiver.recv(),if data_open && jobs.len()<32=>match event {Some(event)=>{if seen.accept(&event.connection,&event.event_id){let bot=brain.clone();jobs.spawn(async move{bot.handle_event(event).await;});}},None=>data_open=false},
                result=jobs.join_next(),if !jobs.is_empty()=>{if let Some(Err(error))=result{log::error!("ingress task: {error}");}},
                else=>break
            }
        }
    });
    let signal = wait_for_shutdown();
    tokio::pin!(signal);
    let mut failure = None;
    while !runners.is_empty() {
        tokio::select! {
            _=&mut signal=>{log::info!("shutting down");SHUTTING_DOWN.store(true,Ordering::SeqCst);let _=shutdown_tx.send(true);break;},
            result=runners.join_next()=>match result {Some(Ok((connection,result)))=>{let _=bot.adapters.lock().unwrap_or_else(|e|e.into_inner()).set_status(&connection,super::outgoing::ConnectionStatus::Disconnected);if let Err(error)=result{log::error!("connection [{connection}]: {error}");failure=Some(error);}},Some(Err(error))=>{log::error!("adapter task: {error}");failure=Some(error.into());},None=>break}
        }
    }
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    let _ = shutdown_tx.send(true);
    while let Some(result) = runners.join_next().await {
        if let Ok((connection, result)) = result {
            let _ = bot
                .adapters
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_status(&connection, super::outgoing::ConnectionStatus::Disconnected);
            if let Err(error) = result {
                failure = Some(error);
            }
        }
    }
    // Dropping the processor's JoinSet aborts model requests after shutdown.
    if !processor.is_finished() {
        processor.abort();
    }
    if let Err(error) = (&mut processor).await {
        if !error.is_cancelled() {
            return Err(error.into());
        }
    }
    if let Some(error) = failure {
        Err(error)
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_dedup_is_bounded_and_connection_qualified() {
        let mut seen = SeenEvents::new(2);
        let first = ConnectionId::new("one");
        let second = ConnectionId::new("two");
        let event = EventKey::discord(1);
        assert!(seen.accept(&first, &event));
        assert!(!seen.accept(&first, &event));
        assert!(seen.accept(&second, &event));
        assert!(seen.accept(&first, &EventKey::discord(2)));
        assert!(seen.accept(&first, &event));
    }
    #[tokio::test]
    async fn ingress_disconnect_is_reported() {
        let (sender, receiver) = EventSender::channel(1);
        drop(receiver);
        assert!(matches!(
            sender
                .send(EventEnvelope {
                    connection: ConnectionId::new("one"),
                    event_id: EventKey::discord(1),
                    occurred_at_unix_ms: 0,
                    event: ChatEvent::ConnectionClosed
                })
                .await,
            Err(PlatformError::Disconnected)
        ));
    }
    #[tokio::test]
    async fn lifecycle_control_is_not_blocked_by_full_message_queue() {
        let (mut sender, mut data) = EventSender::channel(1);
        let (control, mut lifecycle) = mpsc::channel(1);
        sender.control = Some(control);
        let event = || EventEnvelope {
            connection: ConnectionId::new("one"),
            event_id: EventKey::discord(1),
            occurred_at_unix_ms: 0,
            event: ChatEvent::ConnectionClosed,
        };
        sender.data.send(event()).await.unwrap();
        sender.send(event()).await.unwrap();
        assert!(matches!(
            lifecycle.recv().await.unwrap().event,
            ChatEvent::ConnectionClosed
        ));
        assert!(data.recv().await.is_some());
    }
}

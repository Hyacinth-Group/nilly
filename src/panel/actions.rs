//! Authenticated adapters for the shared command schema. No native interaction is forged.
use super::routes::{self, WebError, WebResult};
use super::views::{Choice, Item, Language, Section};
use super::App;
use crate::command::{self, ActionArgs, ActionError, ActionId, ActionTarget, ActionValue, Actor};
use crate::platform::outgoing::{ConnectionSnapshot, ConnectionStatus, RoomSnapshot};
use crate::platform::types::{ConnectionId, PlatformKind, RoomKey, SpaceKey, ThreadKey};
use crate::Bot;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const RETENTION: Duration = Duration::from_secs(30 * 60);
const JOB_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CAPACITY: usize = 128;
const OUTSTANDING: usize = 10;
const CONCURRENCY: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Interrupted,
}
impl JobState {
    fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Interrupted)
    }
    fn key(self) -> &'static str {
        match self {
            Self::Queued => "panel.actions.queued",
            Self::Running => "panel.actions.running",
            Self::Completed => "panel.actions.completed",
            Self::Failed => "panel.actions.failed",
            Self::Interrupted => "panel.actions.interrupted",
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct Job {
    pub id: String,
    pub action: String,
    pub state: JobState,
    pub created: i64,
    pub started: Option<i64>,
    pub finished: Option<i64>,
    pub result: Option<Value>,
    pub error: Option<SafeFailure>,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct SafeFailure {
    pub code: &'static str,
    pub message_key: &'static str,
}
impl SafeFailure {
    fn new(code: &'static str, message_key: &'static str) -> Self {
        Self { code, message_key }
    }
    fn web(&self, status: StatusCode) -> WebError {
        WebError::new(status, self.code, self.message_key)
    }
}
struct Entry {
    owner: [u8; 32],
    key: String,
    fingerprint: [u8; 32],
    touched: Instant,
    job: Job,
    abort: Option<tokio::task::AbortHandle>,
}
#[derive(Default)]
struct Store {
    stopping: bool,
    entries: HashMap<String, Entry>,
}
pub(super) struct JobStore {
    store: Mutex<Store>,
    workers: Arc<Semaphore>,
    timeout: Duration,
}
fn digest(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}
fn opaque() -> String {
    let mut bytes = [0; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[usize::from(byte >> 4)] as char);
        value.push(HEX[usize::from(byte & 15)] as char);
    }
    value
}
impl JobStore {
    pub(super) fn new() -> Self {
        Self {
            store: Mutex::new(Store::default()),
            workers: Arc::new(Semaphore::new(CONCURRENCY)),
            timeout: JOB_TIMEOUT,
        }
    }
    fn prune(store: &mut Store) {
        store
            .entries
            .retain(|_, entry| !entry.job.state.terminal() || entry.touched.elapsed() < RETENTION);
    }
    fn get(&self, owner: &str, id: &str) -> Option<Job> {
        let mut store = self.store.lock();
        Self::prune(&mut store);
        store
            .entries
            .get(id)
            .filter(|entry| entry.owner == digest(owner.as_bytes()))
            .map(|entry| entry.job.clone())
    }
    fn replay(
        &self,
        owner: &str,
        key: &str,
        fingerprint: [u8; 32],
    ) -> Result<Option<Job>, SafeFailure> {
        let mut store = self.store.lock();
        Self::prune(&mut store);
        if store.stopping {
            return Err(SafeFailure::new("stopping", "panel.state.stopping"));
        }
        let owner = digest(owner.as_bytes());
        if let Some(entry) = store
            .entries
            .values()
            .find(|entry| entry.owner == owner && entry.key == key)
        {
            if entry.fingerprint != fingerprint {
                return Err(SafeFailure::new(
                    "idempotency_conflict",
                    "panel.actions.replay_conflict",
                ));
            }
            return Ok(Some(entry.job.clone()));
        }
        Ok(None)
    }
    fn submit<F>(
        self: &Arc<Self>,
        owner: &str,
        key: &str,
        fingerprint: [u8; 32],
        action: &str,
        work: F,
    ) -> Result<Job, SafeFailure>
    where
        F: Future<Output = Result<Value, SafeFailure>> + Send + 'static,
    {
        let job = {
            let mut store = self.store.lock();
            Self::prune(&mut store);
            if store.stopping {
                return Err(SafeFailure::new("stopping", "panel.state.stopping"));
            }
            let owner = digest(owner.as_bytes());
            if let Some(entry) = store
                .entries
                .values()
                .find(|entry| entry.owner == owner && entry.key == key)
            {
                if entry.fingerprint != fingerprint {
                    return Err(SafeFailure::new(
                        "idempotency_conflict",
                        "panel.actions.replay_conflict",
                    ));
                }
                return Ok(entry.job.clone());
            }
            if store.entries.len() >= CAPACITY
                || store
                    .entries
                    .values()
                    .filter(|entry| !entry.job.state.terminal())
                    .count()
                    >= OUTSTANDING
            {
                return Err(SafeFailure::new(
                    "job_queue_full",
                    "panel.actions.queue_full",
                ));
            }
            let job = Job {
                id: opaque(),
                action: action.to_owned(),
                state: JobState::Queued,
                created: crate::now_unix(),
                started: None,
                finished: None,
                result: None,
                error: None,
            };
            store.entries.insert(
                job.id.clone(),
                Entry {
                    owner,
                    key: key.into(),
                    fingerprint,
                    touched: Instant::now(),
                    job: job.clone(),
                    abort: None,
                },
            );
            job
        };
        let worker_store = self.clone();
        let id = job.id.clone();
        let worker = tokio::spawn(async move {
            let _permit = worker_store
                .workers
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| SafeFailure::new("interrupted", "panel.actions.interrupted"))?;
            {
                let mut store = worker_store.store.lock();
                if store.stopping {
                    return Err(SafeFailure::new("interrupted", "panel.actions.interrupted"));
                }
                if let Some(entry) = store.entries.get_mut(&id) {
                    entry.job.state = JobState::Running;
                    entry.job.started = Some(crate::now_unix());
                }
            }
            tokio::time::timeout(worker_store.timeout, work)
                .await
                .unwrap_or_else(|_| Err(SafeFailure::new("job_timeout", "panel.actions.timeout")))
        });
        {
            let mut store = self.store.lock();
            if store.stopping {
                worker.abort();
            } else if let Some(entry) = store.entries.get_mut(&job.id) {
                entry.abort = Some(worker.abort_handle());
            }
        }
        let monitor_store = self.clone();
        let id = job.id.clone();
        tokio::spawn(async move {
            let result = worker.await.unwrap_or_else(|error| {
                Err(if error.is_cancelled() {
                    SafeFailure::new("interrupted", "panel.actions.interrupted")
                } else {
                    SafeFailure::new("job_failed", "panel.actions.failed")
                })
            });
            let mut store = monitor_store.store.lock();
            if let Some(entry) = store.entries.get_mut(&id) {
                if entry.job.state.terminal() {
                    return;
                }
                entry.job.finished = Some(crate::now_unix());
                entry.touched = Instant::now();
                entry.abort = None;
                match result {
                    Ok(result) => {
                        entry.job.state = JobState::Completed;
                        entry.job.result = Some(result);
                    }
                    Err(error) => {
                        entry.job.state = if error.code == "interrupted" {
                            JobState::Interrupted
                        } else {
                            JobState::Failed
                        };
                        entry.job.error = Some(error);
                    }
                }
            }
        });
        Ok(job)
    }
    /// Call before stopping adapters. In-memory jobs are never replayed across restarts.
    pub(super) fn shutdown(&self) {
        let mut store = self.store.lock();
        store.stopping = true;
        self.workers.close();
        for entry in store
            .entries
            .values_mut()
            .filter(|entry| !entry.job.state.terminal())
        {
            if let Some(abort) = entry.abort.take() {
                abort.abort();
            }
            entry.job.state = JobState::Interrupted;
            entry.job.finished = Some(crate::now_unix());
            entry.job.error = Some(SafeFailure::new("interrupted", "panel.actions.interrupted"));
            entry.touched = Instant::now();
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Destination {
    platform: PlatformKind,
    connection_id: ConnectionId,
    space_key: Option<SpaceKey>,
    room_key: RoomKey,
    thread_key: Option<ThreadKey>,
}
impl Destination {
    fn from_room(connection: &ConnectionSnapshot, room: &RoomSnapshot) -> Self {
        Self {
            platform: connection.platform,
            connection_id: connection.connection.clone(),
            space_key: room.room.space.clone(),
            room_key: room.room.id.clone(),
            thread_key: room.room.thread.clone(),
        }
    }
}
fn snapshots(bot: &Bot) -> Vec<ConnectionSnapshot> {
    let registry = bot
        .adapters
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut snapshots = registry.snapshot();
    for connection in &mut snapshots {
        connection.rooms.retain(|room| {
            registry.allows(&connection.connection, &room.room)
                && registry.is_bound(&connection.connection, &room.room)
        });
    }
    snapshots
}
fn resolve(
    connections: &[ConnectionSnapshot],
    destination: &Destination,
) -> Result<ActionTarget, SafeFailure> {
    let connection = connections
        .iter()
        .find(|connection| {
            connection.connection == destination.connection_id
                && connection.platform == destination.platform
                && connection.status == ConnectionStatus::Ready
        })
        .ok_or_else(|| {
            SafeFailure::new(
                "destination_unavailable",
                "panel.actions.destination_unavailable",
            )
        })?;
    let room = connection
        .rooms
        .iter()
        .find(|room| {
            room.room.id == destination.room_key
                && room.room.space == destination.space_key
                && room.room.thread == destination.thread_key
        })
        .ok_or_else(|| {
            SafeFailure::new(
                "destination_forbidden",
                "panel.actions.destination_forbidden",
            )
        })?;
    if room.room.thread.is_some() && !room.capabilities.threads {
        return Err(SafeFailure::new(
            "unsupported_thread",
            "panel.actions.unsupported",
        ));
    }
    Ok(ActionTarget {
        connection: Some(connection.connection.clone()),
        room: Some(room.room.clone()),
    })
}
fn capability(args: &ActionArgs, room: &RoomSnapshot) -> bool {
    (!matches!(
        args,
        ActionArgs::News {} | ActionArgs::Problem {} | ActionArgs::Prank {} | ActionArgs::Hack {}
    ) || room.capabilities.send)
        && (!matches!(args, ActionArgs::Prank {} | ActionArgs::Hack {})
            || (room.capabilities.send_attachments
                && room.capabilities.upload_bytes.is_none_or(|size| size > 0)))
        && (room.room.thread.is_none() || room.capabilities.threads)
}
fn effects(args: &ActionArgs) -> Vec<&'static str> {
    let mut result = Vec::with_capacity(5);
    if matches!(
        args,
        ActionArgs::News {}
            | ActionArgs::Problem {}
            | ActionArgs::Prank {}
            | ActionArgs::Hack {}
            | ActionArgs::Wake {}
    ) {
        result.push("posts");
    }
    if matches!(
        args,
        ActionArgs::Mind { test: true } | ActionArgs::Agents {}
    ) {
        result.push("memory");
    }
    if matches!(
        args,
        ActionArgs::Mind { test: true }
            | ActionArgs::News {}
            | ActionArgs::Problem {}
            | ActionArgs::Wander {}
            | ActionArgs::Prank {}
            | ActionArgs::Hack {}
            | ActionArgs::Agents {}
            | ActionArgs::Wake {}
    ) {
        result.push("may_cost");
    }
    if !args.requires_room() && !is_read(args) {
        result.push("global");
    }
    if matches!(
        args,
        ActionArgs::Debug { .. } | ActionArgs::Mind { test: true }
    ) {
        result.push("privacy");
    }
    result
}
fn is_read(args: &ActionArgs) -> bool {
    matches!(
        args,
        ActionArgs::Status {}
            | ActionArgs::Help {}
            | ActionArgs::Mind { test: false }
            | ActionArgs::Settings {}
            | ActionArgs::Thinking { mode: None }
            | ActionArgs::Model { id: None }
    )
}
fn read_href(args: &ActionArgs) -> Option<&'static str> {
    match args {
        ActionArgs::Status {}
        | ActionArgs::Thinking { mode: None }
        | ActionArgs::Model { id: None } => Some("/"),
        ActionArgs::Help {} => Some("/help"),
        ActionArgs::Mind { test: false } => Some("/memory"),
        ActionArgs::Settings {} => Some("/settings"),
        _ => None,
    }
}
fn parse_args(id: ActionId, values: &HashMap<String, ActionValue>) -> WebResult<ActionArgs> {
    // Stable API option keys only; localized native aliases are not HTTP protocol.
    let definition = command::schema::definition(id);
    if values
        .keys()
        .any(|key| !definition.options.iter().any(|option| option.key == key))
    {
        return Err(WebError::validation());
    }
    if matches!(values.get("hours"), Some(ActionValue::Integer(hours)) if !(1..=24).contains(hours))
    {
        return Err(WebError::validation());
    }
    if let Some(ActionValue::String(model)) = values.get("id") {
        if model.is_empty()
            || model.trim() != model
            || model.len() > 256
            || model.chars().any(char::is_control)
        {
            return Err(WebError::validation());
        }
    }
    if id == ActionId::Debug && !values.contains_key("state") {
        return Err(WebError::validation());
    }
    command::parser::parse_values(id, values).map_err(|_| WebError::validation())
}
fn action_id(raw: &str) -> WebResult<ActionId> {
    ActionId::ALL
        .into_iter()
        .find(|id| id.as_str() == raw)
        .ok_or_else(|| {
            WebError::new(
                StatusCode::NOT_FOUND,
                "unknown_action",
                "panel.actions.unknown",
            )
        })
}
fn failure(error: ActionError) -> SafeFailure {
    match error {
        ActionError::InvalidArgument { .. } => {
            SafeFailure::new("invalid_argument", "panel.error.validation")
        }
        ActionError::UnknownAction => SafeFailure::new("unknown_action", "panel.actions.unknown"),
        ActionError::PermissionDenied
        | ActionError::UnboundRoom
        | ActionError::PrivateSurfaceRequired => SafeFailure::new(
            "destination_forbidden",
            "panel.actions.destination_forbidden",
        ),
        ActionError::MissingRoom | ActionError::MissingConnection => {
            SafeFailure::new("destination_required", "panel.actions.destination_required")
        }
        ActionError::Memory(_) => {
            SafeFailure::new("storage_failed", "panel.actions.storage_failed")
        }
        ActionError::Platform(_) => {
            SafeFailure::new("platform_failed", "panel.actions.platform_failed")
        }
        ActionError::Operation(_) => {
            SafeFailure::new("operation_failed", "panel.actions.operation_failed")
        }
        ActionError::Busy => SafeFailure::new("conversation_busy", "panel.actions.busy"),
        ActionError::ModelValidationUnsupported => SafeFailure::new(
            "model_validation_unsupported",
            "panel.actions.model_validation_unsupported",
        ),
    }
}
async fn photos(app: &App) -> WebResult<bool> {
    let permit = app.tests.clone().try_acquire_owned().map_err(|_| {
        WebError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "panel.actions.queue_full",
        )
    })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::agents::random_image().is_some()
    })
    .await
    .map_err(|_| {
        WebError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "photo_check_failed",
            "panel.actions.photos_unavailable",
        )
    })
}
fn unavailable(
    args: &ActionArgs,
    bot: Option<&Bot>,
    connections: &[ConnectionSnapshot],
    has_photos: bool,
) -> Option<&'static str> {
    if bot.is_none() {
        return Some("panel.actions.bot_unavailable");
    }
    if matches!(args, ActionArgs::Model { id: Some(_) })
        && bot.is_some_and(|bot| !command::AdminService::new(bot).model_validation_supported())
    {
        return Some("panel.actions.model_validation_unsupported");
    }
    if matches!(args, ActionArgs::Prank {} | ActionArgs::Hack {}) && !has_photos {
        return Some("panel.actions.photos_unavailable");
    }
    if args.requires_room()
        && !connections
            .iter()
            .filter(|connection| connection.status == ConnectionStatus::Ready)
            .any(|connection| connection.rooms.iter().any(|room| capability(args, room)))
    {
        return Some("panel.actions.destination_unavailable");
    }
    None
}

pub(super) fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/actions", get(view))
        .route("/ui/actions/{action_id}", post(ui_submit))
        .route("/jobs/{job_id}", get(job_view))
        .route("/api/v1/actions", get(catalog))
        .route(
            "/api/v1/actions/{action_id}",
            get(read_api).post(api_submit),
        )
        .route("/api/v1/jobs/{job_id}", get(job_api))
        .route("/api/v1/connections", get(connections_api))
        .route(
            "/api/v1/connections/{connection_id}/spaces",
            get(spaces_api),
        )
        .route(
            "/api/v1/connections/{connection_id}/conversations",
            get(conversations_api),
        )
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    #[default]
    Execute,
    UseDefaultModel,
}
fn submission_effects(args: &ActionArgs, operation: Operation) -> Vec<&'static str> {
    if operation == Operation::UseDefaultModel {
        vec!["global"]
    } else {
        effects(args)
    }
}
fn validate_operation(id: ActionId, input: &Submission) -> WebResult<()> {
    if input.operation == Operation::UseDefaultModel
        && (id != ActionId::Model || !input.args.is_empty() || input.destination.is_some())
    {
        return Err(WebError::validation());
    }
    if let Some(destination) = &input.destination {
        if destination.connection_id.as_str().len() > 128
            || destination.room_key.as_str().len() > 512
            || destination
                .space_key
                .as_ref()
                .is_some_and(|key| key.as_str().len() > 512)
            || destination
                .thread_key
                .as_ref()
                .is_some_and(|key| key.as_str().len() > 512)
        {
            return Err(WebError::validation());
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    csrf: String,
    idempotency_key: String,
    #[serde(default)]
    args: HashMap<String, ActionValue>,
    destination: Option<Destination>,
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    operation: Operation,
}
fn fingerprint(
    id: ActionId,
    args: &ActionArgs,
    destination: &Option<Destination>,
    operation: Operation,
) -> [u8; 32] {
    // Typed args have deterministic field order, independent of submitted map order.
    digest(
        serde_json::to_string(&(id, args, destination, operation))
            .expect("action DTO serializes")
            .as_bytes(),
    )
}
fn valid_key(key: &str) -> bool {
    (16..=128).contains(&key.len())
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
async fn submit(app: Arc<App>, owner: &str, id: ActionId, input: Submission) -> WebResult<Job> {
    if !valid_key(&input.idempotency_key) {
        return Err(WebError::validation());
    }
    validate_operation(id, &input)?;
    let args = parse_args(id, &input.args)?;
    if input.operation == Operation::Execute && is_read(&args) {
        return Err(WebError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "read_action",
            "panel.actions.read_action",
        ));
    }
    if !args.requires_room() && input.destination.is_some() {
        return Err(WebError::validation());
    }
    if !submission_effects(&args, input.operation).is_empty() && !input.confirmed {
        return Err(WebError::new(
            StatusCode::CONFLICT,
            "confirmation_required",
            "panel.actions.confirm_required",
        ));
    }
    let fingerprint = fingerprint(id, &args, &input.destination, input.operation);
    if let Some(job) = app
        .jobs
        .replay(owner, &input.idempotency_key, fingerprint)
        .map_err(|error| error.web(StatusCode::CONFLICT))?
    {
        return Ok(job);
    }
    let bot = app.runtime.lock().bot.clone().ok_or_else(|| {
        WebError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "bot_unavailable",
            "panel.actions.bot_unavailable",
        )
    })?;
    let has_photos = if matches!(args, ActionArgs::Prank {} | ActionArgs::Hack {}) {
        photos(&app).await?
    } else {
        true
    };
    validate_target(&bot, &args, input.destination.as_ref(), has_photos)
        .map_err(|error| error.web(StatusCode::UNPROCESSABLE_ENTITY))?;
    if input.operation == Operation::UseDefaultModel
        && !command::AdminService::new(&bot).model_validation_supported()
    {
        return Err(WebError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "model_validation_unsupported",
            "panel.actions.model_validation_unsupported",
        ));
    }
    let jobs = app.jobs.clone();
    let destination = input.destination;
    let operation = input.operation;
    jobs.submit(
        owner,
        &input.idempotency_key,
        fingerprint,
        id.as_str(),
        async move {
            // Recheck policy/capabilities after waiting in the queue, not just at HTTP ingress.
            let target = validate_target(&bot, &args, destination.as_ref(), has_photos)?;
            let service = command::AdminService::new_owned(&bot);
            let result = if operation == Operation::UseDefaultModel {
                service.use_default_model(&Actor::LocalOperator).await
            } else {
                service.execute(&Actor::LocalOperator, &target, args).await
            }
            .map_err(failure)?;
            let result = serde_json::to_value(result)
                .map_err(|_| SafeFailure::new("job_failed", "panel.actions.failed"))?;
            if serde_json::to_vec(&result)
                .map_err(|_| SafeFailure::new("job_failed", "panel.actions.failed"))?
                .len()
                > 64 * 1024
            {
                return Err(SafeFailure::new(
                    "result_too_large",
                    "panel.actions.result_too_large",
                ));
            }
            Ok(result)
        },
    )
    .map_err(|error| error.web(StatusCode::TOO_MANY_REQUESTS))
}
fn validate_target(
    bot: &Bot,
    args: &ActionArgs,
    destination: Option<&Destination>,
    has_photos: bool,
) -> Result<ActionTarget, SafeFailure> {
    let connections = snapshots(bot);
    if let Some(reason) = unavailable(args, Some(bot), &connections, has_photos) {
        return Err(SafeFailure::new("action_unavailable", reason));
    }
    let target = if let Some(destination) = destination {
        let target = resolve(&connections, destination)?;
        let room = connections
            .iter()
            .find(|connection| connection.connection == destination.connection_id)
            .and_then(|connection| {
                connection.rooms.iter().find(|room| {
                    room.room.conversation()
                        == target.room.as_ref().expect("resolved room").conversation()
                })
            })
            .expect("resolved snapshot");
        if !capability(args, room) {
            return Err(SafeFailure::new(
                "capability_unavailable",
                "panel.actions.unsupported",
            ));
        }
        target
    } else if args.requires_room() {
        return Err(SafeFailure::new(
            "destination_required",
            "panel.actions.destination_required",
        ));
    } else {
        ActionTarget::default()
    };
    command::AdminService::new(bot)
        .authorize(&Actor::LocalOperator, &target, args)
        .map_err(failure)?;
    Ok(target)
}
async fn read_api(
    State(app): State<Arc<App>>,
    Path(raw): Path<String>,
    headers: HeaderMap,
) -> WebResult<Json<Value>> {
    routes::admin(&app, &headers)?;
    let args = default_args(action_id(&raw)?);
    if !is_read(&args) {
        return Err(WebError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "mutation_requires_post",
            "panel.actions.mutation_requires_post",
        ));
    }
    let bot = app.runtime.lock().bot.clone().ok_or_else(|| {
        WebError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "bot_unavailable",
            "panel.actions.bot_unavailable",
        )
    })?;
    let result = command::AdminService::new_owned(&bot)
        .execute(&Actor::LocalOperator, &ActionTarget::default(), args)
        .await
        .map_err(|error| failure(error).web(StatusCode::SERVICE_UNAVAILABLE))?;
    Ok(Json(
        serde_json::to_value(result).map_err(|_| WebError::validation())?,
    ))
}
async fn api_submit(
    State(app): State<Arc<App>>,
    Path(raw): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Submission>,
) -> WebResult<Response> {
    let owner = routes::mutation(&app, &headers, &input.csrf)?;
    let job = submit(app, &owner, action_id(&raw)?, input).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"job_id":job.id,"state":job.state,"status_url":format!("/api/v1/jobs/{}",job.id)}))).into_response())
}
async fn job_api(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Json<Value>> {
    let owner = routes::token(&headers)?;
    app.auth.require_admin_passive(&owner)?;
    let job = app.jobs.get(&owner, &id).ok_or_else(|| {
        WebError::new(
            StatusCode::NOT_FOUND,
            "job_not_found",
            "panel.actions.job_missing",
        )
    })?;
    let mut value = serde_json::to_value(&job).expect("job DTO serializes");
    let language = routes::lang(&query);
    value["message"] = json!(routes::t(language, job.state.key()));
    value["display"] = json!({
        "state":routes::t(language,job.state.key()),
        "result":job.result.as_ref().map(|result|serde_json::to_string_pretty(result).expect("result DTO serializes")).unwrap_or_else(||routes::t(language,"panel.actions.pending_result")),
        "error":job.error.as_ref().map(|error|routes::t(language,error.message_key)).unwrap_or_default()
    });
    if let Some(error) = job.error {
        value["error"]["message"] = json!(routes::t(language, error.message_key));
    }
    Ok(Json(value))
}
async fn connections_api(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> WebResult<Json<Value>> {
    routes::admin(&app, &headers)?;
    let bot = app.runtime.lock().bot.clone();
    Ok(Json(
        json!({"connections": bot.as_ref().map_or_else(Vec::new, |bot| snapshots(bot))}),
    ))
}
async fn spaces_api(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> WebResult<Json<Value>> {
    routes::admin(&app, &headers)?;
    let bot = app.runtime.lock().bot.clone();
    let connections = bot.as_ref().map_or_else(Vec::new, |bot| snapshots(bot));
    let connection = connections
        .iter()
        .find(|connection| connection.connection.as_str() == id)
        .ok_or_else(|| {
            WebError::new(
                StatusCode::NOT_FOUND,
                "connection_not_found",
                "panel.actions.destination_unavailable",
            )
        })?;
    let mut seen = HashSet::new();
    let spaces: Vec<_> = connection
        .rooms
        .iter()
        .filter_map(|room| room.room.space.clone())
        .filter(|space| seen.insert(space.clone()))
        .collect();
    let named = bot
        .as_ref()
        .map(|bot| {
            bot.adapters
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .discovered_spaces()
        })
        .unwrap_or_default();
    Ok(Json(
        json!({"spaces":spaces.iter().map(|space| json!({"space_key":space,"name":named.iter().find(|(connection,found)|connection.as_str()==id&&found.id==*space).and_then(|(_,found)|found.name.as_deref())})).collect::<Vec<_>>()}),
    ))
}
async fn conversations_api(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Json<Value>> {
    routes::admin(&app, &headers)?;
    let bot = app.runtime.lock().bot.clone();
    let connections = bot.as_ref().map_or_else(Vec::new, |bot| snapshots(bot));
    let connection = connections
        .iter()
        .find(|connection| connection.connection.as_str() == id)
        .ok_or_else(|| {
            WebError::new(
                StatusCode::NOT_FOUND,
                "connection_not_found",
                "panel.actions.destination_unavailable",
            )
        })?;
    if query.get("space").is_some_and(|space| space.len() > 512) {
        return Err(WebError::validation());
    }
    Ok(Json(
        json!({"conversations":connection.rooms.iter().filter(|room|query.get("space").is_none_or(|space|room.room.space.as_ref().is_some_and(|key|key.as_str()==space))).map(|room|json!({"destination":Destination::from_room(connection,room),"name":room.room.name,"capabilities":room.capabilities,"ready":connection.status==ConnectionStatus::Ready})).collect::<Vec<_>>()}),
    ))
}

fn default_args(id: ActionId) -> ActionArgs {
    match id {
        ActionId::Debug => ActionArgs::Debug {
            setting: Some(command::DebugSetting::Off),
        },
        _ => command::parser::parse_values(id, &HashMap::new()).expect("schema defaults"),
    }
}
fn variants(id: ActionId) -> Vec<ActionArgs> {
    match id {
        ActionId::Mind => vec![
            ActionArgs::Mind { test: false },
            ActionArgs::Mind { test: true },
        ],
        ActionId::Reset => vec![
            ActionArgs::Reset { all: false },
            ActionArgs::Reset { all: true },
        ],
        ActionId::Thinking => std::iter::once(ActionArgs::Thinking { mode: None })
            .chain(
                [
                    command::ThinkingSetting::Show,
                    command::ThinkingSetting::Hide,
                    command::ThinkingSetting::Silent,
                    command::ThinkingSetting::Off,
                ]
                .into_iter()
                .map(|mode| ActionArgs::Thinking { mode: Some(mode) }),
            )
            .collect(),
        ActionId::Debug => vec![
            ActionArgs::Debug {
                setting: Some(command::DebugSetting::On),
            },
            ActionArgs::Debug {
                setting: Some(command::DebugSetting::Off),
            },
        ],
        _ => vec![default_args(id)],
    }
}
fn submission_args(args: &ActionArgs) -> Value {
    match args {
        ActionArgs::Mind { test } => json!({"test":test}),
        ActionArgs::Reset { all } => json!({"all":all}),
        ActionArgs::Sleep { hours } => json!({"hours":hours}),
        ActionArgs::Thinking { mode: Some(mode) } => {
            json!({"mode":command::schema::definition(ActionId::Thinking).options[0].choices[*mode as usize].value})
        }
        ActionArgs::Model { id: Some(id) } => json!({"id":id}),
        ActionArgs::Debug {
            setting: Some(setting),
        } => {
            json!({"state":command::schema::definition(ActionId::Debug).options[0].choices[*setting as usize].value})
        }
        _ => json!({}),
    }
}
fn option_schema(id: ActionId, language: Language) -> Vec<Value> {
    command::schema::definition(id).options.iter().map(|option| {
        let default = match option.default {
            Some(command::schema::OptionDefault::Boolean(value)) => json!(value),
            Some(command::schema::OptionDefault::Integer(value)) => json!(value),
            None => Value::Null,
        };
        json!({
            "key":option.key, "label":routes::t(language,&format!("panel.actions.option.{}.{}",id.as_str(),option.key)),
            "kind":match option.kind {command::schema::OptionKind::Boolean=>"boolean",command::schema::OptionKind::Integer=>"integer",command::schema::OptionKind::String=>"string"},
            "default":default, "min":option.min, "max":if id==ActionId::Sleep {Some(24)} else {option.max},
            "choices":option.choices.iter().map(|choice|json!({"value":choice.value,"label":routes::t(language,&format!("panel.actions.choice.{}",choice.value))})).collect::<Vec<_>>()
        })
    }).collect()
}
async fn catalog(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Json<Value>> {
    routes::admin(&app, &headers)?;
    let language = routes::lang(&query);
    let bot = app.runtime.lock().bot.clone();
    let connections = bot.as_ref().map_or_else(Vec::new, |bot| snapshots(bot));
    let has_photos = if bot.is_some() {
        photos(&app).await?
    } else {
        false
    };
    Ok(Json(json!({
        "actions":command::schema::definitions().iter().map(|definition|json!({
            "id":definition.id,"label":routes::t(language,&format!("panel.actions.card.{}",definition.id.as_str())),
            "description":routes::t(language,&format!("panel.actions.description.{}",definition.id.as_str())),
            "options":option_schema(definition.id,language),
            "variants":variants(definition.id).iter().map(|args|{
                let reason=if read_href(args).is_some() {None} else {unavailable(args,bot.as_deref(),&connections,has_photos)};
                json!({"args":submission_args(args),"typed_args":args,"read":is_read(args),"href":read_href(args),"api_url":format!("/api/v1/actions/{}",definition.id.as_str()),"method":if is_read(args) {"GET"} else {"POST"},"requires_destination":args.requires_room(),"side_effects":effects(args),"available":reason.is_none(),"unavailable_reason":reason.map(|key|routes::t(language,key))})
            }).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "read_variants":{"thinking":{"href":"/","api_url":"/api/v1/actions/thinking"},"model":{"href":"/","api_url":"/api/v1/actions/model"}},
        "setter_variants":{"thinking":{"mode_choices":option_schema(ActionId::Thinking,language),"available":bot.is_some()},"model":{"id_required":true,"available":bot.as_ref().is_some_and(|bot|command::AdminService::new(bot).model_validation_supported()),"unavailable_reason":if bot.is_none() {Some(routes::t(language,"panel.actions.bot_unavailable"))} else if !bot.as_ref().is_some_and(|bot|command::AdminService::new(bot).model_validation_supported()) {Some(routes::t(language,"panel.actions.model_validation_unsupported"))} else {None}},"debug":{"state_required":true,"available":bot.is_some()}},
        "default_model_restore":{"action_id":"model","operation":"use_default_model","available":bot.as_ref().is_some_and(|bot|command::AdminService::new(bot).model_validation_supported()),"side_effects":["global"],"requires_confirmation":true},
        "jobs":{"capacity":CAPACITY,"outstanding_limit":OUTSTANDING,"concurrency":CONCURRENCY,"retention_seconds":RETENTION.as_secs(),"timeout_seconds":JOB_TIMEOUT.as_secs(),"replayed_after_restart":false}
    })))
}
fn destination_label(connection: &ConnectionSnapshot, room: &RoomSnapshot) -> String {
    format!(
        "{} / {} / {} / {} ({}) / {}",
        connection.platform.as_str(),
        connection.connection,
        room.room
            .space
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        room.room.name.as_deref().unwrap_or(""),
        room.room.id,
        room.room
            .thread
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default()
    )
}
async fn view(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let owner = routes::admin(&app, &headers)?;
    let language = routes::lang(&query);
    let csrf = app.auth.session(&owner)?.csrf;
    let bot = app.runtime.lock().bot.clone();
    let connections = bot.as_ref().map_or_else(Vec::new, |bot| snapshots(bot));
    let has_photos = if bot.is_some() {
        photos(&app).await?
    } else {
        false
    };
    let mut page = routes::page(language, "panel.actions.title", "actions")?;
    page.authenticated = true;
    page.description = routes::t(language, "panel.actions.explanation");
    for definition in command::schema::definitions() {
        let id = definition.id;
        let args = default_args(id);
        let mut section = routes::section(language, &format!("panel.actions.card.{}", id.as_str()));
        section.id = if id == ActionId::Agents {
            "refresh_agents".into()
        } else {
            id.as_str().into()
        };
        section.description = routes::t(
            language,
            &format!("panel.actions.description.{}", id.as_str()),
        );
        if let Some(href) = read_href(&args) {
            section.items.push(Item {
                label: section.title.clone(),
                value: routes::t(language, "panel.actions.open"),
                href: format!("{href}?lang={}", language.code()),
                ..Item::default()
            });
        }
        if matches!(id, ActionId::Status | ActionId::Help | ActionId::Settings) {
            page.sections.push(section);
            continue;
        }
        let form_args = if id == ActionId::Mind {
            ActionArgs::Mind { test: true }
        } else {
            args.clone()
        };
        if let Some(reason) = unavailable(&form_args, bot.as_deref(), &connections, has_photos) {
            section.items.push(Item {
                label: routes::t(language, "panel.actions.unavailable"),
                value: routes::t(language, reason),
                ..Item::default()
            });
            if id != ActionId::Reset || bot.is_none() {
                page.sections.push(section);
                continue;
            }
        }
        if id == ActionId::Model
            && bot
                .as_ref()
                .is_some_and(|bot| !command::AdminService::new(bot).model_validation_supported())
        {
            section.items.push(Item {
                label: routes::t(language, "panel.actions.unavailable"),
                value: routes::t(language, "panel.actions.model_validation_unsupported"),
                ..Item::default()
            });
            page.sections.push(section);
            continue;
        }
        let card_effects = if matches!(id, ActionId::Thinking | ActionId::Model) {
            vec!["global"]
        } else {
            effects(&form_args)
        };
        for effect in card_effects {
            section.items.push(Item {
                label: routes::t(language, &format!("panel.actions.effect.{effect}")),
                ..Item::default()
            });
        }
        let mut fields = Vec::new();
        for option in definition.options {
            let mut field = routes::field(
                language,
                option.key,
                &format!("panel.actions.option.{}.{}", id.as_str(), option.key),
                "text",
                "",
            );
            field.help = routes::t(
                language,
                &format!("panel.actions.description.{}", id.as_str()),
            );
            match option.kind {
                command::schema::OptionKind::Boolean => {
                    field.kind = "select".into();
                    field.choices = vec![
                        Choice {
                            value: "false".into(),
                            label: routes::t(
                                language,
                                if id == ActionId::Reset {
                                    "panel.actions.reset_one"
                                } else {
                                    "panel.actions.browse_memory"
                                },
                            ),
                            selected: true,
                        },
                        Choice {
                            value: "true".into(),
                            label: routes::t(
                                language,
                                if id == ActionId::Reset {
                                    "panel.actions.reset_all"
                                } else {
                                    "panel.actions.test_memory"
                                },
                            ),
                            selected: false,
                        },
                    ];
                }
                command::schema::OptionKind::Integer => {
                    field.kind = "number".into();
                    field.value = command::schema::default_sleep_hours().to_string();
                    field.required = true;
                }
                command::schema::OptionKind::String if !option.choices.is_empty() => {
                    field.kind = "select".into();
                    if id == ActionId::Thinking {
                        field.choices.push(Choice {
                            value: String::new(),
                            label: routes::t(language, "panel.actions.read_current"),
                            selected: true,
                        });
                    }
                    field
                        .choices
                        .extend(option.choices.iter().map(|choice| Choice {
                            value: choice.value.into(),
                            label: routes::t(
                                language,
                                &format!("panel.actions.choice.{}", choice.value),
                            ),
                            selected: false,
                        }));
                }
                command::schema::OptionKind::String => {
                    field.help = routes::t(language, "panel.actions.model_help");
                }
            }
            fields.push(field);
        }
        if id == ActionId::Model {
            fields.push(super::views::Field {
                name: "operation".into(),
                label: routes::t(language, "panel.actions.model_operation"),
                kind: "select".into(),
                choices: vec![
                    Choice {
                        value: "execute".into(),
                        label: routes::t(language, "panel.actions.model_read_set"),
                        selected: true,
                    },
                    Choice {
                        value: "use_default_model".into(),
                        label: routes::t(language, "panel.actions.default_model"),
                        selected: false,
                    },
                ],
                ..super::views::Field::default()
            });
        }
        if matches!(
            id,
            ActionId::Mind
                | ActionId::Reset
                | ActionId::News
                | ActionId::Problem
                | ActionId::Prank
                | ActionId::Hack
        ) {
            let mut picker = routes::field(
                language,
                "destination",
                "panel.actions.destination",
                "select",
                "",
            );
            picker.choices.push(Choice {
                value: String::new(),
                label: routes::t(language, "panel.actions.choose_destination"),
                selected: true,
            });
            picker.choices.extend(
                connections
                    .iter()
                    .filter(|connection| connection.status == ConnectionStatus::Ready)
                    .flat_map(|connection| {
                        connection
                            .rooms
                            .iter()
                            .filter(|room| capability(&form_args, room))
                            .map(move |room| Choice {
                                value: serde_json::to_string(&Destination::from_room(
                                    connection, room,
                                ))
                                .expect("destination DTO serializes"),
                                label: destination_label(connection, room),
                                selected: false,
                            })
                    }),
            );
            picker.help = routes::t(language, "panel.actions.destination_help");
            fields.push(picker);
        }
        let mut form = routes::form(
            &format!("action-{}", id.as_str()),
            &format!("/ui/actions/{}", id.as_str()),
            &csrf,
            fields,
            vec![routes::button(language, "panel.actions.review")],
        );
        form.hidden.push(("idempotency_key".into(), opaque()));
        section.forms.push(form);
        page.sections.push(section);
    }
    let mut policy = routes::section(language, "panel.actions.retention");
    policy.description = routes::t(language, "panel.actions.retention_help");
    page.sections.push(policy);
    routes::html(&page)
}
fn form_submission(id: ActionId, form: &HashMap<String, String>) -> WebResult<Submission> {
    let mut args = HashMap::new();
    let definition = command::schema::definition(id);
    for (key, value) in form {
        if [
            "csrf",
            "lang",
            "destination",
            "idempotency_key",
            "confirmed",
            "operation",
        ]
        .contains(&key.as_str())
        {
            continue;
        }
        let option = definition
            .options
            .iter()
            .find(|option| option.key == key)
            .ok_or_else(WebError::validation)?;
        if value.is_empty() && matches!(option.kind, command::schema::OptionKind::String) {
            continue;
        }
        let value = match option.kind {
            command::schema::OptionKind::Boolean => {
                ActionValue::Boolean(value.parse().map_err(|_| WebError::validation())?)
            }
            command::schema::OptionKind::Integer => {
                ActionValue::Integer(value.parse().map_err(|_| WebError::validation())?)
            }
            command::schema::OptionKind::String => ActionValue::String(value.clone()),
        };
        args.insert(key.clone(), value);
    }
    let destination = form
        .get("destination")
        .filter(|value| !value.is_empty())
        .map(|value| serde_json::from_str(value).map_err(|_| WebError::validation()))
        .transpose()?;
    let destination = if parse_args(id, &args)?.requires_room() {
        destination
    } else {
        None
    };
    let operation = match form.get("operation").map(String::as_str) {
        None | Some("execute") => Operation::Execute,
        Some("use_default_model") => Operation::UseDefaultModel,
        _ => return Err(WebError::validation()),
    };
    Ok(Submission {
        csrf: form.get("csrf").cloned().unwrap_or_default(),
        idempotency_key: form.get("idempotency_key").cloned().unwrap_or_default(),
        args,
        destination,
        confirmed: form.get("confirmed").is_some_and(|value| value == "true"),
        operation,
    })
}
async fn ui_submit(
    State(app): State<Arc<App>>,
    Path(raw): Path<String>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> WebResult<Response> {
    let language = routes::lang(&form);
    let owner = routes::mutation(&app, &headers, form.get("csrf").map_or("", String::as_str))?;
    let id = action_id(&raw)?;
    let input = form_submission(id, &form)?;
    let args = parse_args(id, &input.args)?;
    validate_operation(id, &input)?;
    if input.operation == Operation::Execute {
        if let Some(href) = read_href(&args) {
            return Ok(Redirect::to(&format!("{href}?lang={}", language.code())).into_response());
        }
    }
    if !valid_key(&input.idempotency_key) {
        return Err(WebError::validation());
    }
    if !submission_effects(&args, input.operation).is_empty() && !input.confirmed {
        let bot = app.runtime.lock().bot.clone().ok_or_else(|| {
            WebError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "bot_unavailable",
                "panel.actions.bot_unavailable",
            )
        })?;
        let has_photos = if matches!(args, ActionArgs::Prank {} | ActionArgs::Hack {}) {
            photos(&app).await?
        } else {
            true
        };
        let target = validate_target(&bot, &args, input.destination.as_ref(), has_photos)
            .map_err(|error| error.web(StatusCode::UNPROCESSABLE_ENTITY))?;
        let mut page = routes::page(language, "panel.actions.confirm", "actions")?;
        page.authenticated = true;
        page.description = routes::t(language, "panel.actions.confirm_help");
        let mut section = routes::section(language, &format!("panel.actions.card.{}", id.as_str()));
        section.items.push(Item {
            label: routes::t(language, "panel.actions.arguments"),
            value: serde_json::to_string(&args).expect("args DTO serializes"),
            ..Item::default()
        });
        section.items.push(Item {
            label: routes::t(language, "panel.actions.model_operation"),
            value: routes::t(
                language,
                if input.operation == Operation::UseDefaultModel {
                    "panel.actions.default_model"
                } else {
                    "panel.actions.execute"
                },
            ),
            ..Item::default()
        });
        let destination = if let Some(room) = target.room.as_ref() {
            format!(
                "{} / {} / {} / {} ({}) / {}",
                room.id.platform().as_str(),
                target
                    .connection
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                room.space
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                room.name.as_deref().unwrap_or(""),
                room.id,
                room.thread
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            )
        } else {
            routes::t(language, "panel.actions.global_destination")
        };
        section.items.push(Item {
            label: routes::t(language, "panel.actions.destination"),
            value: destination,
            ..Item::default()
        });
        if matches!(args, ActionArgs::Wake {}) {
            let candidates = {
                let state = bot.state();
                [
                    state
                        .pending_mentions
                        .last()
                        .map(|(conversation, _, _)| conversation.clone()),
                    state.last_channel.clone(),
                ]
            };
            let connections = snapshots(&bot);
            let mut seen = HashSet::new();
            for conversation in candidates
                .into_iter()
                .flatten()
                .filter(|conversation| seen.insert(conversation.clone()))
            {
                let destination = connections
                    .iter()
                    .find_map(|connection| {
                        connection
                            .rooms
                            .iter()
                            .find(|room| room.room.conversation() == conversation)
                            .map(|room| destination_label(connection, room))
                    })
                    .unwrap_or_else(|| {
                        format!(
                            "{} / {}",
                            conversation.room,
                            conversation
                                .thread
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_default()
                        )
                    });
                section.items.push(Item {
                    label: routes::t(language, "panel.actions.automatic_destination"),
                    value: destination,
                    detail: routes::t(language, "panel.actions.automatic_destination_help"),
                    ..Item::default()
                });
            }
        }
        for effect in submission_effects(&args, input.operation) {
            section.items.push(Item {
                label: routes::t(language, &format!("panel.actions.effect.{effect}")),
                ..Item::default()
            });
        }
        let mut button = routes::button(language, "panel.actions.confirm_execute");
        button.name = "confirmed".into();
        button.value = "true".into();
        let mut confirmation = routes::form(
            "action-confirm",
            &format!("/ui/actions/{}", id.as_str()),
            &input.csrf,
            Vec::new(),
            vec![button],
        );
        confirmation.hidden = form
            .into_iter()
            .filter(|(key, _)| !["csrf", "lang", "confirmed"].contains(&key.as_str()))
            .collect();
        section.forms.push(confirmation);
        section.items.push(Item {
            label: routes::t(language, "panel.actions.cancel"),
            value: routes::t(language, "panel.actions.cancel"),
            href: format!("/actions?lang={}", language.code()),
            ..Item::default()
        });
        page.sections.push(section);
        return routes::html(&page);
    }
    let job = submit(app, &owner, id, input).await?;
    Ok(Redirect::to(&format!("/jobs/{}?lang={}", job.id, language.code())).into_response())
}
async fn job_view(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let owner = routes::admin(&app, &headers)?;
    let language = routes::lang(&query);
    let job = app.jobs.get(&owner, &id).ok_or_else(|| {
        WebError::new(
            StatusCode::NOT_FOUND,
            "job_not_found",
            "panel.actions.job_missing",
        )
    })?;
    let mut page = routes::page(language, "panel.actions.job_title", "actions")?;
    page.authenticated = true;
    page.description = routes::t(language, "panel.actions.job_help");
    if !job.state.terminal() {
        page.poll_url = format!("/api/v1/jobs/{id}?lang={}", language.code());
    }
    let mut section = Section {
        title: routes::t(language, &format!("panel.actions.card.{}", job.action)),
        ..Section::default()
    };
    section.items.push(Item {
        label: routes::t(language, "panel.actions.job_state"),
        value: routes::t(language, job.state.key()),
        key: "display.state".into(),
        ..Item::default()
    });
    section.items.push(Item {
        label: routes::t(language, "panel.actions.created"),
        value: job.created.to_string(),
        ..Item::default()
    });
    section.items.push(Item {
        label: routes::t(language, "panel.actions.error"),
        value: job
            .error
            .as_ref()
            .map(|error| routes::t(language, error.message_key))
            .unwrap_or_default(),
        key: "display.error".into(),
        ..Item::default()
    });
    section.items.push(Item {
        label: routes::t(language, "panel.actions.result"),
        value: job
            .result
            .as_ref()
            .map(|result| serde_json::to_string_pretty(result).expect("result DTO serializes"))
            .unwrap_or_else(|| routes::t(language, "panel.actions.pending_result")),
        key: "display.result".into(),
        ..Item::default()
    });
    section.items.push(Item {
        label: routes::t(language, "panel.actions.refresh"),
        value: routes::t(language, "panel.actions.refresh"),
        href: format!("/jobs/{id}?lang={}", language.code()),
        ..Item::default()
    });
    section.items.push(Item {
        label: routes::t(language, "panel.actions.title"),
        value: routes::t(language, "panel.actions.title"),
        href: format!("/actions?lang={}", language.code()),
        ..Item::default()
    });
    page.sections.push(section);
    routes::html(&page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;
    use tower::ServiceExt;

    async fn terminal(store: &JobStore, owner: &str, id: &str) -> Job {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let job = store.get(owner, id).unwrap();
                if job.state.terminal() {
                    return job;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
    #[test]
    fn all_sixteen_cards_and_schema_options_are_covered() {
        assert_eq!(command::schema::definitions().len(), 16);
        for id in ActionId::ALL {
            assert_eq!(action_id(id.as_str()).ok(), Some(id));
            assert_eq!(default_args(id).id(), id);
            assert!(!variants(id).is_empty());
            assert_eq!(
                option_schema(id, Language::En).len(),
                command::schema::definition(id).options.len()
            );
            for language in [Language::En, Language::Tr] {
                assert!(super::super::views::text(
                    language,
                    &format!("panel.actions.card.{}", id.as_str())
                )
                .is_ok());
                assert!(super::super::views::text(
                    language,
                    &format!("panel.actions.description.{}", id.as_str())
                )
                .is_ok());
            }
            for args in variants(id) {
                let values =
                    serde_json::from_value::<HashMap<String, ActionValue>>(submission_args(&args))
                        .unwrap();
                assert_eq!(parse_args(id, &values).unwrap(), args);
            }
        }
        assert!(action_id("post_arbitrary").is_err());
    }
    #[test]
    fn sleep_and_mode_arguments_use_shared_types_with_panel_bounds() {
        for hours in [1, 8, 24] {
            assert!(
                matches!(parse_args(ActionId::Sleep,&HashMap::from([("hours".into(),ActionValue::Integer(hours))])).unwrap(),ActionArgs::Sleep {hours:value} if value==hours)
            );
        }
        for hours in [i64::MIN, 0, 25, i64::MAX] {
            assert!(parse_args(
                ActionId::Sleep,
                &HashMap::from([("hours".into(), ActionValue::Integer(hours))])
            )
            .is_err());
        }
        for mode in ["goster", "gizle", "sessiz", "kapat"] {
            assert!(parse_args(
                ActionId::Thinking,
                &HashMap::from([("mode".into(), ActionValue::String(mode.into()))])
            )
            .is_ok());
        }
        assert!(parse_args(
            ActionId::Thinking,
            &HashMap::from([("mode".into(), ActionValue::String("invented".into()))])
        )
        .is_err());
        assert!(parse_args(
            ActionId::Sleep,
            &HashMap::from([("hours".into(), ActionValue::String("8".into()))])
        )
        .is_err());
        assert!(parse_args(ActionId::Debug, &HashMap::new()).is_err());
        assert!(parse_args(
            ActionId::Model,
            &HashMap::from([("id".into(), ActionValue::String("secret\nvalue".into()))])
        )
        .is_err());
    }
    #[test]
    fn reset_memory_and_read_setter_variants_are_distinct() {
        assert!(variants(ActionId::Reset)[0].requires_room());
        assert!(!variants(ActionId::Reset)[1].requires_room());
        assert!(is_read(&variants(ActionId::Mind)[0]));
        assert!(variants(ActionId::Mind)[1].requires_room());
        assert!(effects(&variants(ActionId::Mind)[1]).contains(&"memory"));
        assert!(is_read(&default_args(ActionId::Thinking)));
        assert!(is_read(&default_args(ActionId::Model)));
        assert!(effects(&ActionArgs::Debug {
            setting: Some(command::DebugSetting::On)
        })
        .contains(&"privacy"));
        assert_eq!(
            submission_effects(&default_args(ActionId::Model), Operation::UseDefaultModel),
            ["global"]
        );
    }
    fn connection() -> ConnectionSnapshot {
        let mut capabilities = crate::platform::capabilities::Capabilities::unavailable();
        capabilities.send = true;
        capabilities.send_attachments = true;
        capabilities.threads = true;
        capabilities.upload_bytes = Some(1024);
        ConnectionSnapshot {
            connection: ConnectionId::new("telegram-main"),
            platform: PlatformKind::Telegram,
            status: ConnectionStatus::Ready,
            account: None,
            warnings: vec![],
            rooms: vec![RoomSnapshot {
                room: crate::platform::types::RoomRef {
                    id: RoomKey::new(PlatformKind::Telegram, &["-123"]).unwrap(),
                    space: None,
                    kind: crate::platform::types::RoomKind::Group,
                    name: Some("Real room".into()),
                    thread: Some(ThreadKey::new(PlatformKind::Telegram, &["-123", "7"]).unwrap()),
                },
                capabilities,
            }],
        }
    }
    #[test]
    fn target_resolves_only_exact_authoritative_thread_and_capabilities() {
        let connection = connection();
        let destination = Destination::from_room(&connection, &connection.rooms[0]);
        let target = resolve(std::slice::from_ref(&connection), &destination).unwrap();
        assert_eq!(target.room.unwrap().name.as_deref(), Some("Real room"));
        let mut forged = destination.clone();
        forged.thread_key = None;
        assert!(resolve(std::slice::from_ref(&connection), &forged).is_err());
        forged = destination.clone();
        forged.connection_id = ConnectionId::new("other");
        assert!(resolve(std::slice::from_ref(&connection), &forged).is_err());
        forged = destination.clone();
        forged.space_key = Some(SpaceKey::new(PlatformKind::Telegram, &["other"]).unwrap());
        assert!(resolve(std::slice::from_ref(&connection), &forged).is_err());
        let mut disconnected = connection.clone();
        disconnected.status = ConnectionStatus::Disconnected;
        assert!(resolve(&[disconnected], &destination).is_err());
        let mut room = connection.rooms[0].clone();
        room.capabilities.send = false;
        assert!(!capability(&ActionArgs::News {}, &room));
        room.capabilities.send = true;
        room.capabilities.upload_bytes = None;
        assert!(capability(&ActionArgs::Prank {}, &room));
        room.capabilities.threads = false;
        assert!(!capability(&ActionArgs::Reset { all: false }, &room));
        assert!(serde_json::from_value::<Destination>(json!({"platform":"telegram","connection_id":"telegram-main","space_key":null,"room_key":"telegram:room:-123","thread_key":null,"name":"Forged"})).is_err());
    }
    #[test]
    fn prank_and_hack_require_attachment_permission_not_a_known_upload_ceiling() {
        for args in [ActionArgs::Prank {}, ActionArgs::Hack {}] {
            let mut room = connection().rooms.remove(0);
            assert!(capability(&args, &room));
            room.capabilities.upload_bytes = None;
            assert!(capability(&args, &room));
            room.capabilities.upload_bytes = Some(0);
            assert!(!capability(&args, &room));
            room.capabilities.upload_bytes = Some(1024);
            assert!(capability(&args, &room));
            room.capabilities.send_attachments = false;
            assert!(!capability(&args, &room));
            room.capabilities.upload_bytes = None;
            assert!(!capability(&args, &room));
            room.capabilities.send_attachments = true;
            room.capabilities.send = false;
            assert!(!capability(&args, &room));
        }
    }
    #[test]
    fn idempotency_is_canonical_and_restoration_has_no_sentinel() {
        let args = ActionArgs::Model { id: None };
        assert_ne!(
            fingerprint(ActionId::Model, &args, &None, Operation::Execute),
            fingerprint(ActionId::Model, &args, &None, Operation::UseDefaultModel)
        );
        let mut input = Submission {
            csrf: String::new(),
            idempotency_key: opaque(),
            args: HashMap::new(),
            destination: None,
            confirmed: true,
            operation: Operation::UseDefaultModel,
        };
        assert!(validate_operation(ActionId::Model, &input).is_ok());
        assert!(validate_operation(ActionId::Sleep, &input).is_err());
        input
            .args
            .insert("id".into(), ActionValue::String("default".into()));
        assert!(validate_operation(ActionId::Model, &input).is_err());
        assert!(!valid_key("short"));
        assert!(!valid_key(&"x".repeat(129)));
        assert!(valid_key(&opaque()));
    }
    #[tokio::test]
    async fn duplicate_submission_does_not_execute_twice_and_session_isolated() {
        let store = Arc::new(JobStore::new());
        let count = Arc::new(AtomicUsize::new(0));
        let first_count = count.clone();
        let first = store
            .submit("owner", "submission-000001", [1; 32], "news", async move {
                first_count.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"delivered":true}))
            })
            .unwrap();
        let second_count = count.clone();
        let duplicate = store
            .submit("owner", "submission-000001", [1; 32], "news", async move {
                second_count.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Null)
            })
            .unwrap();
        assert_eq!(first.id, duplicate.id);
        assert!(store.get("other", &first.id).is_none());
        assert!(store
            .submit("owner", "submission-000001", [2; 32], "news", async {
                Ok(Value::Null)
            })
            .is_err());
        assert_eq!(
            terminal(&store, "owner", &first.id).await.state,
            JobState::Completed
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let other = store
            .submit("other", "submission-000001", [1; 32], "news", async {
                Ok(Value::Null)
            })
            .unwrap();
        assert_ne!(first.id, other.id);
        store.shutdown();
    }
    #[tokio::test]
    async fn queue_and_concurrency_are_bounded_and_disconnect_does_not_cancel() {
        let store = Arc::new(JobStore::new());
        let barrier = Arc::new(Notify::new());
        let started = Arc::new(AtomicUsize::new(0));
        let mut ids = Vec::new();
        for index in 0..OUTSTANDING {
            let barrier = barrier.clone();
            let started = started.clone();
            ids.push(
                store
                    .submit(
                        "owner",
                        &format!("submission-{index:06}"),
                        [1; 32],
                        "news",
                        async move {
                            started.fetch_add(1, Ordering::SeqCst);
                            barrier.notified().await;
                            Ok(json!({"delivered":true}))
                        },
                    )
                    .unwrap()
                    .id,
            );
        }
        assert!(store
            .submit("owner", "submission-extra", [1; 32], "news", async {
                Ok(Value::Null)
            })
            .is_err());
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert_eq!(started.load(Ordering::SeqCst), CONCURRENCY);
        assert_eq!(
            store
                .store
                .lock()
                .entries
                .values()
                .filter(|entry| entry.job.state == JobState::Queued)
                .count(),
            OUTSTANDING - CONCURRENCY
        );
        let running: Vec<_> = store
            .store
            .lock()
            .entries
            .values()
            .filter(|entry| entry.job.state == JobState::Running)
            .map(|entry| entry.job.id.clone())
            .collect();
        barrier.notify_waiters();
        for id in &running {
            assert_eq!(
                terminal(&store, "owner", id).await.state,
                JobState::Completed
            );
        }
        store.shutdown();
        for id in ids.iter().filter(|id| !running.contains(id)) {
            assert_eq!(store.get("owner", id).unwrap().state, JobState::Interrupted);
        }
    }
    #[tokio::test]
    async fn failure_panic_and_shutdown_are_not_success() {
        let store = Arc::new(JobStore::new());
        let failed = store
            .submit("owner", "submission-failure", [1; 32], "news", async {
                Err(SafeFailure::new(
                    "platform_failed",
                    "panel.actions.platform_failed",
                ))
            })
            .unwrap();
        assert_eq!(
            terminal(&store, "owner", &failed.id).await.state,
            JobState::Failed
        );
        let panicked = store
            .submit("owner", "submission-panic00", [1; 32], "news", async {
                panic!("test task panic");
                #[allow(unreachable_code)]
                Ok(Value::Null)
            })
            .unwrap();
        assert_eq!(
            terminal(&store, "owner", &panicked.id).await.state,
            JobState::Failed
        );
        let unfinished = store
            .submit(
                "owner",
                "submission-pending",
                [1; 32],
                "news",
                std::future::pending(),
            )
            .unwrap();
        store.shutdown();
        assert_eq!(
            store.get("owner", &unfinished.id).unwrap().state,
            JobState::Interrupted
        );
        assert!(store
            .submit("owner", "submission-stopped", [1; 32], "news", async {
                Ok(Value::Null)
            })
            .is_err());
    }
    #[tokio::test]
    async fn timed_out_work_is_a_failure_not_a_success() {
        let store = Arc::new(JobStore {
            store: Mutex::new(Store::default()),
            workers: Arc::new(Semaphore::new(CONCURRENCY)),
            timeout: Duration::from_millis(5),
        });
        let job = store
            .submit(
                "owner",
                "submission-timeout",
                [1; 32],
                "news",
                std::future::pending(),
            )
            .unwrap();
        let job = terminal(&store, "owner", &job.id).await;
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.error.unwrap().code, "job_timeout");
    }
    #[tokio::test]
    async fn retained_replays_are_bounded_and_expire_without_restart_replay() {
        let store = Arc::new(JobStore::new());
        {
            let mut inner = store.store.lock();
            for index in 0..CAPACITY {
                let id = index.to_string();
                inner.entries.insert(
                    id.clone(),
                    Entry {
                        owner: digest(b"owner"),
                        key: format!("submission-{index:06}"),
                        fingerprint: [1; 32],
                        touched: Instant::now(),
                        abort: None,
                        job: Job {
                            id,
                            action: "news".into(),
                            state: JobState::Completed,
                            created: 0,
                            started: None,
                            finished: Some(0),
                            result: None,
                            error: None,
                        },
                    },
                );
            }
        }
        assert!(store
            .submit("owner", "submission-extra", [1; 32], "news", async {
                Ok(Value::Null)
            })
            .is_err());
        assert!(store
            .replay("owner", "submission-000000", [1; 32])
            .unwrap()
            .is_some());
        store.store.lock().entries.get_mut("0").unwrap().touched =
            Instant::now() - RETENTION - Duration::from_secs(1);
        assert!(store.get("owner", "0").is_none());
        assert!(Arc::new(JobStore::new())
            .replay("owner", "submission-000000", [1; 32])
            .unwrap()
            .is_none());
        store.shutdown();
    }
    #[tokio::test]
    async fn http_catalog_picker_jobs_require_auth_and_mutations_require_csrf() {
        let (app, _directory) = super::super::tests::fixture().await;
        let owner = super::super::tests::owner(&app).await;
        let router = routes::router(app.clone());
        for path in [
            "/actions",
            "/api/v1/actions",
            "/api/v1/connections",
            "/api/v1/connections/test/spaces",
            "/api/v1/connections/test/conversations",
            "/api/v1/jobs/test",
            "/jobs/test",
        ] {
            assert_eq!(
                router
                    .clone()
                    .oneshot(super::super::tests::request("GET", path, None, ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        let catalog = router
            .clone()
            .oneshot(super::super::tests::request(
                "GET",
                "/api/v1/actions",
                Some(&owner.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(catalog.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(catalog.into_body(), 128 * 1024)
            .await
            .unwrap();
        let catalog: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(catalog["actions"].as_array().unwrap().len(), 16);
        let body=json!({"csrf":owner.info.csrf,"idempotency_key":"submission-offline","args":{},"confirmed":true}).to_string();
        let mut offline =
            super::super::tests::request("POST", "/api/v1/actions/wake", Some(&owner.token), &body);
        offline.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        assert_eq!(
            router.clone().oneshot(offline).await.unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        let body=json!({"csrf":owner.info.csrf,"idempotency_key":"submission-read00","args":{},"confirmed":true}).to_string();
        let mut readonly = super::super::tests::request(
            "POST",
            "/api/v1/actions/status",
            Some(&owner.token),
            &body,
        );
        readonly.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        assert_eq!(
            router.clone().oneshot(readonly).await.unwrap().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        let body=json!({"csrf":"invalid","idempotency_key":"submission-000001","args":{},"confirmed":true}).to_string();
        let mut request =
            super::super::tests::request("POST", "/api/v1/actions/wake", Some(&owner.token), &body);
        request.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        assert_eq!(
            router.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let mut request=super::super::tests::request("POST","/api/v1/actions/wake",Some(&owner.token),&json!({"csrf":owner.info.csrf,"idempotency_key":"submission-000001","args":{},"confirmed":true}).to_string());
        request.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        request.headers_mut().insert(
            axum::http::header::ORIGIN,
            "https://attacker.invalid".parse().unwrap(),
        );
        assert_eq!(
            router.oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}

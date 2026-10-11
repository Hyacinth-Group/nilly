use super::routes::{
    admin, button, field, form, html, lang, mutation, page, section, t, WebError, WebResult,
};
use super::views::{Item, Language, Page};
use super::{auth, App};
use crate::memory::{self as store, MutationError, Person, RecordSnapshot};
use crate::platform::{
    keys,
    types::{encode_component, UserKey},
};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

const MAX_CONTENT: usize = 24 * 1024;
const MAX_PAGE: usize = 100;

pub(super) fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/memory", get(browse_ui))
        .route("/memory/{kind}/{id}", get(detail_ui))
        .route(
            "/ui/memory/{kind}/{id}",
            post(save_ui).layer(DefaultBodyLimit::max(96 * 1024)),
        )
        .route("/api/v1/memory/{kind}", get(browse_api))
        .route("/api/v1/memory/{kind}/{id}", get(detail_api).put(save_api))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    People,
    Topics,
    Events,
}
impl Kind {
    fn parse(value: &str) -> WebResult<Self> {
        match value {
            "people" => Ok(Self::People),
            "topics" => Ok(Self::Topics),
            "events" => Ok(Self::Events),
            _ => Err(invalid_identity()),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::People => "people",
            Self::Topics => "topics",
            Self::Events => "events",
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Self::People => "kisiler/",
            Self::Topics => "konular/",
            Self::Events => "olaylar/",
        }
    }
    fn threshold(self) -> usize {
        match self {
            Self::People => store::PERSON_LIMIT,
            Self::Topics => store::TOPIC_LIMIT,
            Self::Events => store::EVENT_LIMIT,
        }
    }
    fn key(self, id: &str) -> WebResult<String> {
        if id.is_empty() || id.len() > 1024 {
            return Err(invalid_identity());
        }
        match self {
            Self::People => {
                let identity: UserKey = id.parse().map_err(|_| invalid_identity())?;
                Ok(keys::person_record(&identity))
            }
            Self::Topics => {
                if id.len() > 256 || store::slug(id) != id {
                    return Err(invalid_identity());
                }
                Ok(format!("{}{id}.md", self.prefix()))
            }
            Self::Events => {
                let bytes = id.as_bytes();
                if bytes.len() != 7
                    || bytes[4] != b'-'
                    || !bytes[..4]
                        .iter()
                        .chain(bytes[5..].iter())
                        .all(u8::is_ascii_digit)
                    || &id[..4] == "0000"
                    || !(1..=12).contains(&id[5..].parse::<u8>().unwrap_or(0))
                {
                    return Err(invalid_identity());
                }
                Ok(format!("{}{id}.md", self.prefix()))
            }
        }
    }
    fn id(self, key: &str) -> WebResult<String> {
        let id = key
            .strip_prefix(self.prefix())
            .and_then(|key| key.strip_suffix(".md"))
            .ok_or_else(invalid_identity)?;
        if self.key(id)? != key {
            return Err(invalid_identity());
        }
        Ok(id.to_owned())
    }
}
fn invalid_identity() -> WebError {
    WebError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_memory_identity",
        "panel.memory.invalid_identity",
    )
}
fn storage_error() -> WebError {
    WebError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "memory_storage_failed",
        "panel.memory.storage_failed",
    )
}
fn missing() -> WebError {
    WebError::new(
        StatusCode::NOT_FOUND,
        "memory_not_found",
        "panel.memory.not_found",
    )
}
fn unsafe_person() -> WebError {
    WebError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "unsafe_person_format",
        "panel.memory.unsafe_person",
    )
}

async fn blocking<T: Send + 'static>(
    app: &App,
    work: impl FnOnce() -> WebResult<T> + Send + 'static,
) -> WebResult<T> {
    let permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| storage_error())?
}
fn favorite(app: &App) -> Option<UserKey> {
    let config = app.config.lock();
    crate::configured_owner(Some(config.get("OWNER_KEY")), Some(config.get("OWNER_ID")))
        .ok()
        .flatten()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersonInput {
    name: String,
    username: String,
    previous_names: Vec<String>,
    score: i32,
    tags: Vec<String>,
    note: String,
    facts: Vec<String>,
    events: Vec<String>,
}
impl PersonInput {
    fn from_person(person: &Person) -> Self {
        Self {
            name: person.name.clone(),
            username: person.username.clone(),
            previous_names: person.previous_names.clone(),
            score: person.score,
            tags: person.tags.clone(),
            note: person.note.clone(),
            facts: person.facts.clone(),
            events: person.events.clone(),
        }
    }
    fn text(self, id: &UserKey, is_favorite: bool) -> WebResult<String> {
        let single = |value: &str| !value.contains(['\r', '\n', '\0']);
        if !(-10..=10).contains(&self.score)
            || self.name.trim().is_empty()
            || self.tags.len() > 6
            || self.previous_names.len() > 5
            || [&self.name, &self.username, &self.note]
                .into_iter()
                .any(|value| !single(value))
            || self
                .previous_names
                .iter()
                .chain(&self.tags)
                .any(|value| !single(value) || value.contains(',') || value.trim().is_empty())
            || self
                .facts
                .iter()
                .chain(&self.events)
                .any(|value| !single(value) || value.trim().is_empty())
            || (is_favorite && (self.score != 10 || self.note != store::FAVORITE_NOTE))
        {
            return Err(WebError::validation());
        }
        let person = Person {
            id: id.clone(),
            name: self.name,
            username: self.username,
            previous_names: self.previous_names,
            score: self.score,
            tags: self.tags,
            note: self.note,
            facts: self.facts,
            events: self.events,
        };
        let text = person.text();
        validate_content(&text)?;
        // Refuse header/list injection or whitespace-normalization loss.
        let parsed = Person::parse(id, &text);
        if parsed.name != person.name
            || parsed.username != person.username
            || parsed.previous_names != person.previous_names
            || parsed.score != person.score
            || parsed.tags != person.tags
            || parsed.note != person.note
            || parsed.facts != person.facts
            || parsed.events != person.events
        {
            return Err(unsafe_person());
        }
        Ok(text)
    }
}
fn structured_safe(id: &UserKey, text: &str) -> bool {
    let canonical = Person::parse(id, text).text();
    let original = text.trim_end_matches(['\r', '\n']);
    let canonical = canonical.trim_end_matches(['\r', '\n']);
    if original == canonical {
        return true;
    }
    // Schema migration keeps original body bytes. Only a matching legacy Discord
    // numeric header is equivalent to the immutable canonical route identity.
    let Some(numeric) = id.as_str().strip_prefix("discord:user:").filter(|value| {
        !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && (!value.starts_with('0') || *value == "0")
            && value.parse::<u64>().is_ok()
    }) else {
        return false;
    };
    let mut originals = original.split('\n');
    for expected in canonical.split('\n') {
        let Some(actual) = originals.next() else {
            return false;
        };
        if actual != expected
            && !(expected.strip_prefix("id: ") == Some(id.as_str())
                && actual.strip_prefix("id: ") == Some(numeric))
        {
            return false;
        }
    }
    originals.next().is_none()
}
fn validate_content(text: &str) -> WebResult<()> {
    if text.len() > MAX_CONTENT || text.contains('\0') {
        return Err(WebError::validation());
    }
    Ok(())
}

#[derive(Serialize)]
struct Record {
    kind: &'static str,
    id: String,
    content: String,
    revision: u64,
    modified: u64,
    modified_date: String,
    etag: String,
    summary_threshold_bytes: usize,
    structured_editable: bool,
    favorite: bool,
    person: Option<PersonInput>,
}
fn record(kind: Kind, snapshot: RecordSnapshot, owner: Option<&UserKey>) -> WebResult<Record> {
    let id = kind.id(&snapshot.key)?;
    let identity = if kind == Kind::People {
        Some(id.parse::<UserKey>().map_err(|_| invalid_identity())?)
    } else {
        None
    };
    let person = identity
        .as_ref()
        .map(|id| PersonInput::from_person(&Person::parse(id, &snapshot.content)));
    let editable = identity
        .as_ref()
        .is_none_or(|id| structured_safe(id, &snapshot.content));
    Ok(Record {
        kind: kind.name(),
        id,
        revision: snapshot.revision,
        modified: snapshot.modified,
        modified_date: store::date_from_unix(i64::try_from(snapshot.modified).unwrap_or(i64::MAX)),
        etag: etag(snapshot.revision),
        summary_threshold_bytes: kind.threshold(),
        structured_editable: editable,
        favorite: identity.as_ref().is_some_and(|id| Some(id) == owner),
        person,
        content: snapshot.content,
    })
}
fn etag(revision: u64) -> String {
    format!("\"{revision}\"")
}
fn response_etag(mut response: Response, revision: u64) -> WebResult<Response> {
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag(revision)).map_err(|_| storage_error())?,
    );
    Ok(response)
}
fn precondition(headers: &HeaderMap) -> WebResult<u64> {
    let value = headers.get(header::IF_MATCH).ok_or_else(|| {
        WebError::new(
            StatusCode::PRECONDITION_REQUIRED,
            "revision_required",
            "panel.memory.revision_required",
        )
    })?;
    let value = value.to_str().map_err(|_| WebError::validation())?;
    let digits = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(WebError::validation)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(WebError::validation());
    }
    digits.parse().map_err(|_| WebError::validation())
}

fn browse_options(query: &HashMap<String, String>) -> WebResult<(String, Option<String>, usize)> {
    let search = query.get("query").cloned().unwrap_or_default();
    let cursor = query
        .get("cursor")
        .filter(|value| !value.is_empty())
        .cloned();
    let limit = query.get("limit").map_or(Ok(25), |value| {
        value.parse::<usize>().map_err(|_| WebError::validation())
    })?;
    if search.len() > 256
        || search.contains('\0')
        || cursor
            .as_ref()
            .is_some_and(|value| value.len() > 2048 || value.contains('\0'))
        || !(1..=MAX_PAGE).contains(&limit)
    {
        return Err(WebError::validation());
    }
    Ok((search, cursor, limit))
}
async fn browse(
    app: &App,
    kind: Kind,
    query: &HashMap<String, String>,
) -> WebResult<(Vec<Record>, Option<String>)> {
    let (search, cursor, limit) = browse_options(query)?;
    let owner = favorite(app);
    blocking(app, move || {
        let result = store::browse_versioned(kind.prefix(), &search, cursor.as_deref(), limit)
            .map_err(|_| storage_error())?;
        // Unsupported historical keys remain private, never become arbitrary-key links.
        let records = result
            .records
            .into_iter()
            .filter(|snapshot| kind.id(&snapshot.key).is_ok())
            .map(|snapshot| record(kind, snapshot, owner.as_ref()))
            .collect::<WebResult<Vec<_>>>()?;
        Ok((records, result.next_cursor))
    })
    .await
}
async fn read(app: &App, kind: Kind, id: String) -> WebResult<Record> {
    let key = kind.key(&id)?;
    let owner = favorite(app);
    blocking(app, move || {
        let snapshot = store::read_versioned(&key)
            .map_err(|_| storage_error())?
            .ok_or_else(missing)?;
        record(kind, snapshot, owner.as_ref())
    })
    .await
}
async fn browse_api(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path(kind): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Json<serde_json::Value>> {
    admin(&app, &headers)?;
    let kind = Kind::parse(&kind)?;
    let (records, next_cursor) = browse(&app, kind, &query).await?;
    Ok(Json(
        json!({"records":records,"next_cursor":next_cursor,"order":"modified_desc_key_asc","index_read_only":true}),
    ))
}
async fn detail_api(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
) -> WebResult<Response> {
    admin(&app, &headers)?;
    let value = read(&app, Kind::parse(&kind)?, id).await?;
    let revision = value.revision;
    response_etag(Json(value).into_response(), revision)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveInput {
    #[serde(default)]
    csrf: String,
    content: Option<String>,
    person: Option<PersonInput>,
}
enum SaveOutcome {
    Saved(Record),
    Conflict {
        current: Record,
        draft: String,
        expected: u64,
    },
}
async fn replace(
    app: &Arc<App>,
    kind: Kind,
    id: String,
    expected: u64,
    input: SaveInput,
) -> WebResult<SaveOutcome> {
    let key = kind.key(&id)?;
    let owner = favorite(app);
    let worker_app = app.clone();
    blocking(app, move || {
        let draft = match kind {
            Kind::People => {
                if input.content.is_some() {
                    return Err(unsafe_person());
                }
                let identity = id.parse::<UserKey>().map_err(|_| invalid_identity())?;
                input
                    .person
                    .ok_or_else(WebError::validation)?
                    .text(&identity, owner.as_ref() == Some(&identity))?
            }
            _ => {
                if input.person.is_some() {
                    return Err(WebError::validation());
                }
                let content = input.content.ok_or_else(WebError::validation)?;
                validate_content(&content)?;
                content
            }
        };
        let runtime = super::routes::edit_runtime(&worker_app)?;
        let snapshot = store::read_versioned(&key)
            .map_err(|_| storage_error())?
            .ok_or_else(missing)?;
        if snapshot.revision != expected {
            return Ok(SaveOutcome::Conflict {
                current: record(kind, snapshot, owner.as_ref())?,
                draft,
                expected,
            });
        }
        if kind == Kind::People
            && !structured_safe(
                &id.parse().map_err(|_| invalid_identity())?,
                &snapshot.content,
            )
        {
            return Err(unsafe_person());
        }
        match store::replace_versioned(runtime.bot.as_deref(), &key, expected, &draft) {
            Ok(snapshot) => Ok(SaveOutcome::Saved(record(kind, snapshot, owner.as_ref())?)),
            Err(MutationError::Conflict(snapshot)) => Ok(SaveOutcome::Conflict {
                current: record(kind, snapshot, owner.as_ref())?,
                draft,
                expected,
            }),
            Err(MutationError::Missing) => Err(missing()),
            Err(MutationError::Storage(_)) => Err(storage_error()),
        }
    })
    .await
}
async fn save_api(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    Json(input): Json<SaveInput>,
) -> WebResult<Response> {
    let csrf = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(&input.csrf);
    mutation(&app, &headers, csrf)?;
    let expected = precondition(&headers)?;
    match replace(&app, Kind::parse(&kind)?, id, expected, input).await? {
        SaveOutcome::Saved(value) => {
            let revision = value.revision;
            response_etag(Json(json!({"record":value,"message":t(lang(&query),"panel.memory.saved"),"future_learning":true})).into_response(), revision)
        }
        SaveOutcome::Conflict {
            current,
            draft,
            expected,
        } => {
            let revision = current.revision;
            let mut response = StatusCode::PRECONDITION_FAILED.into_response();
            response.extensions_mut().insert(super::routes::ErrorDetails(json!({"code":"revision_conflict","message":t(lang(&query),"panel.memory.conflict"),"field_errors":{},"current":current,"submitted_draft":draft,"submitted_revision":expected})));
            response_etag(response, revision)
        }
    }
}

fn link(language: Language, label: &str, value: String, href: String) -> Item {
    Item {
        label: t(language, label),
        value,
        href,
        ..Default::default()
    }
}
fn base(language: Language, selected: Kind) -> WebResult<Page> {
    let mut result = page(language, "panel.title.memory", "memory")?;
    result.authenticated = true;
    result.description = t(language, "panel.memory.description");
    let mut tabs = section(language, "panel.memory.kinds");
    for kind in [Kind::People, Kind::Topics, Kind::Events] {
        let mut item = link(
            language,
            &format!("panel.memory.{}", kind.name()),
            t(language, &format!("panel.memory.{}", kind.name())),
            format!("/memory?kind={}&lang={}", kind.name(), language.code()),
        );
        item.active = kind == selected;
        tabs.items.push(item);
    }
    result.sections.push(tabs);
    let mut index = section(language, "panel.memory.index");
    index.description = t(language, "panel.memory.index_help");
    result.sections.push(index);
    Ok(result)
}
async fn browse_ui(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    admin(&app, &headers)?;
    let language = lang(&query);
    let kind = Kind::parse(query.get("kind").map_or("people", String::as_str))?;
    let (search, _, limit) = browse_options(&query)?;
    let (records, next_cursor) = browse(&app, kind, &query).await?;
    let mut result = base(language, kind)?;
    let mut search_section = section(language, "panel.memory.search");
    let mut search_form = form(
        "memory-search",
        "/memory",
        "",
        vec![field(
            language,
            "query",
            "panel.memory.query",
            "text",
            &search,
        )],
        vec![button(language, "panel.memory.search")],
    );
    search_form.method = "get".into();
    search_form.hidden = vec![
        ("kind".into(), kind.name().into()),
        ("limit".into(), limit.to_string()),
    ];
    search_section.forms.push(search_form);
    result.sections.push(search_section);
    let mut list = section(language, &format!("panel.memory.{}", kind.name()));
    list.description = t(
        language,
        if records.is_empty() {
            "panel.memory.empty"
        } else {
            "panel.memory.order"
        },
    );
    for record in records {
        let title = record
            .person
            .as_ref()
            .map(|person| person.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                record
                    .content
                    .lines()
                    .find_map(|line| line.strip_prefix("# "))
                    .unwrap_or(&record.id)
                    .to_owned()
            });
        let mut item = link(
            language,
            "panel.memory.record",
            title,
            format!(
                "/memory/{}/{}?lang={}",
                kind.name(),
                encode_component(&record.id),
                language.code()
            ),
        );
        item.detail = format!(
            "{} · {} · {} {}",
            record.id,
            record.modified_date,
            t(language, "panel.memory.revision"),
            record.revision
        );
        list.items.push(item);
    }
    result.sections.push(list);
    if let Some(cursor) = next_cursor {
        let mut pagination = section(language, "panel.memory.pagination");
        pagination.items.push(link(
            language,
            "panel.memory.pagination",
            t(language, "panel.memory.next"),
            format!(
                "/memory?kind={}&query={}&cursor={}&limit={limit}&lang={}",
                kind.name(),
                encode_component(&search),
                encode_component(&cursor),
                language.code()
            ),
        ));
        result.sections.push(pagination);
    }
    html(&result)
}
fn metadata(language: Language, value: &Record) -> super::views::Section {
    let mut result = section(language, "panel.memory.metadata");
    for (label, value) in [
        ("panel.memory.identity", value.id.clone()),
        ("panel.memory.revision", value.revision.to_string()),
        (
            "panel.memory.modified",
            format!("{} (Unix {})", value.modified_date, value.modified),
        ),
        (
            "panel.memory.threshold",
            value.summary_threshold_bytes.to_string(),
        ),
        ("panel.memory.limit", MAX_CONTENT.to_string()),
    ] {
        result.items.push(Item {
            label: t(language, label),
            value,
            ..Default::default()
        });
    }
    result.description = t(language, "panel.memory.limits_help");
    result
}
fn editor(language: Language, value: &Record, csrf: &str) -> super::views::Section {
    let mut result = section(language, "panel.memory.edit");
    result.description = t(
        language,
        if value.kind == "events" {
            "panel.memory.events_help"
        } else {
            "panel.memory.edit_help"
        },
    );
    let mut fields = Vec::new();
    if let Some(person) = &value.person {
        let mut identity = field(
            language,
            "identity",
            "panel.memory.identity",
            "text",
            &value.id,
        );
        identity.read_only = true;
        fields.push(identity);
        for (name, label, kind, text) in [
            ("name", "panel.memory.name", "text", person.name.clone()),
            (
                "username",
                "panel.memory.username",
                "text",
                person.username.clone(),
            ),
            (
                "previous_names",
                "panel.memory.previous_names",
                "textarea",
                person.previous_names.join("\n"),
            ),
            (
                "score",
                "panel.memory.score",
                "number",
                if value.favorite {
                    "10".into()
                } else {
                    person.score.to_string()
                },
            ),
            (
                "tags",
                "panel.memory.tags",
                "textarea",
                person.tags.join("\n"),
            ),
            (
                "note",
                "panel.memory.note",
                "text",
                if value.favorite {
                    store::FAVORITE_NOTE.into()
                } else {
                    person.note.clone()
                },
            ),
            (
                "facts",
                "panel.memory.facts",
                "textarea",
                person.facts.join("\n"),
            ),
            (
                "events",
                "panel.memory.person_events",
                "textarea",
                person.events.join("\n"),
            ),
        ] {
            let mut item = field(language, name, label, kind, &text);
            item.read_only =
                !value.structured_editable || (value.favorite && matches!(name, "score" | "note"));
            item.help = t(
                language,
                if value.favorite && matches!(name, "score" | "note") {
                    "panel.memory.favorite_help"
                } else if name == "score" {
                    "panel.memory.score_help"
                } else if matches!(name, "previous_names" | "tags") {
                    "panel.memory.lines_help"
                } else if matches!(name, "facts" | "events") {
                    "panel.memory.facts_events_help"
                } else if name == "note" {
                    "panel.memory.note_help"
                } else {
                    "panel.memory.person_help"
                },
            );
            fields.push(item);
        }
        if !value.structured_editable {
            result.description = t(language, "panel.memory.unsafe_person");
            let mut raw = field(
                language,
                "raw",
                "panel.memory.raw",
                "textarea",
                &value.content,
            );
            raw.read_only = true;
            fields.push(raw);
        }
    } else {
        fields.push(field(
            language,
            "content",
            "panel.memory.content",
            "textarea",
            &value.content,
        ));
    }
    let buttons = if value.structured_editable {
        vec![button(language, "panel.memory.save")]
    } else {
        Vec::new()
    };
    let mut edit = form(
        "memory-edit",
        &format!("/ui/memory/{}/{}", value.kind, encode_component(&value.id)),
        csrf,
        fields,
        buttons,
    );
    edit.hidden
        .push(("revision".into(), value.revision.to_string()));
    result.forms.push(edit);
    result.items.push(link(
        language,
        "panel.memory.cancel",
        t(language, "panel.memory.cancel"),
        format!("/memory?kind={}&lang={}", value.kind, language.code()),
    ));
    result
}
async fn detail_ui(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let session = admin(&app, &headers)?;
    let csrf = app.auth.session(&session)?.csrf;
    let language = lang(&query);
    let value = read(&app, Kind::parse(&kind)?, id).await?;
    let mut result = base(language, Kind::parse(value.kind)?)?;
    if query.get("saved").is_some_and(|saved| saved == "1") {
        result.notice = t(language, "panel.memory.saved");
    }
    result.sections.push(metadata(language, &value));
    result.sections.push(editor(language, &value, &csrf));
    response_etag(html(&result)?, value.revision)
}
fn input_form(input: &mut HashMap<String, String>, kind: Kind) -> WebResult<SaveInput> {
    let csrf = input.remove("csrf").unwrap_or_default();
    let result = if kind == Kind::People {
        let lines = |value: String| {
            value
                .lines()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect()
        };
        SaveInput {
            csrf,
            content: None,
            person: Some(PersonInput {
                name: input.remove("name").ok_or_else(WebError::validation)?,
                username: input.remove("username").unwrap_or_default(),
                previous_names: lines(input.remove("previous_names").unwrap_or_default()),
                score: input
                    .remove("score")
                    .ok_or_else(WebError::validation)?
                    .parse()
                    .map_err(|_| WebError::validation())?,
                tags: lines(input.remove("tags").unwrap_or_default()),
                note: input.remove("note").unwrap_or_default(),
                facts: lines(input.remove("facts").unwrap_or_default()),
                events: lines(input.remove("events").unwrap_or_default()),
            }),
        }
    } else {
        SaveInput {
            csrf,
            content: Some(input.remove("content").ok_or_else(WebError::validation)?),
            person: None,
        }
    };
    if !input.is_empty() {
        return Err(WebError::validation());
    }
    Ok(result)
}
fn conflict_page(
    language: Language,
    current: &Record,
    draft: &str,
    expected: u64,
) -> WebResult<Page> {
    let mut result = base(language, Kind::parse(current.kind)?)?;
    result.error = t(language, "panel.memory.conflict");
    result.sections.push(metadata(language, current));
    let mut comparison = section(language, "panel.memory.compare");
    comparison.description = t(language, "panel.memory.copy_help");
    let mut draft_field = field(language, "draft", "panel.memory.draft", "textarea", draft);
    draft_field.read_only = true;
    draft_field.help = format!("{} {expected}", t(language, "panel.memory.revision"));
    let mut current_field = field(
        language,
        "current",
        "panel.memory.current",
        "textarea",
        &current.content,
    );
    current_field.read_only = true;
    // No submit buttons and no overwrite endpoint: select/copy works without JavaScript.
    comparison.forms.push(form(
        "memory-conflict",
        "/memory",
        "",
        vec![draft_field, current_field],
        vec![],
    ));
    comparison.items.push(link(
        language,
        "panel.memory.reload",
        t(language, "panel.memory.reload"),
        format!(
            "/memory/{}/{}?lang={}",
            current.kind,
            encode_component(&current.id),
            language.code()
        ),
    ));
    result.sections.push(comparison);
    Ok(result)
}
async fn save_ui(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
    Form(mut input): Form<HashMap<String, String>>,
) -> WebResult<Response> {
    let language = Language::parse(input.remove("lang").as_deref().unwrap_or("en"));
    let csrf = input.get("csrf").cloned().unwrap_or_default();
    mutation(&app, &headers, &csrf)?;
    let expected = input
        .remove("revision")
        .ok_or_else(|| {
            WebError::new(
                StatusCode::PRECONDITION_REQUIRED,
                "revision_required",
                "panel.memory.revision_required",
            )
        })?
        .parse()
        .map_err(|_| WebError::validation())?;
    let kind = Kind::parse(&kind)?;
    if input
        .remove("identity")
        .is_some_and(|identity| identity != id)
    {
        return Err(invalid_identity());
    }
    let input = input_form(&mut input, kind)?;
    match replace(&app, kind, id, expected, input).await? {
        SaveOutcome::Saved(value) => Ok(Redirect::to(&format!(
            "/memory/{}/{}?lang={}&saved=1",
            value.kind,
            encode_component(&value.id),
            language.code()
        ))
        .into_response()),
        SaveOutcome::Conflict {
            current,
            draft,
            expected,
        } => {
            let revision = current.revision;
            let mut response = html(&conflict_page(language, &current, &draft, expected)?)?;
            *response.status_mut() = StatusCode::PRECONDITION_FAILED;
            response
                .extensions_mut()
                .insert(super::routes::PreserveErrorBody);
            response_etag(response, revision)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    fn expect<T>(result: WebResult<T>) -> T {
        match result {
            Ok(value) => value,
            Err(_) => panic!("unexpected web error"),
        }
    }
    fn put(
        path: &str,
        session: &auth::SessionIssue,
        revision: Option<u64>,
        csrf: bool,
        body: serde_json::Value,
    ) -> Request<Body> {
        let mut request =
            super::super::tests::request("PUT", path, Some(&session.token), &body.to_string());
        request.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if let Some(revision) = revision {
            request.headers_mut().insert(
                header::IF_MATCH,
                HeaderValue::from_str(&etag(revision)).unwrap(),
            );
        }
        if csrf {
            request.headers_mut().insert(
                "x-csrf-token",
                HeaderValue::from_str(&session.info.csrf).unwrap(),
            );
        }
        request
    }
    async fn json_body(response: Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
    }
    fn seed(key: &str, text: &str) -> RecordSnapshot {
        store::test::ensure_test_db();
        store::write_result(key, text).unwrap();
        store::read_versioned(key).unwrap().unwrap()
    }

    #[test]
    fn identities_are_canonical_and_paths_are_never_storage_keys() {
        for (kind, invalid) in [
            (Kind::People, "42"),
            (Kind::People, "../huy"),
            (Kind::People, "discord:room:42"),
            (Kind::Topics, "../INDEX"),
            (Kind::Topics, "a/b"),
            (Kind::Topics, "%2E%2E"),
            (Kind::Topics, "huy.md"),
            (Kind::Topics, "UpperCase"),
            (Kind::Events, "2026-00"),
            (Kind::Events, "2026-13"),
            (Kind::Events, "2026-1"),
            (Kind::Events, "0000-01"),
            (Kind::Events, "2026-01/../huy"),
        ] {
            assert!(kind.key(invalid).is_err(), "{kind:?} {invalid}");
        }
        assert_eq!(
            expect(Kind::People.key("discord:user:42")),
            "kisiler/discord:user:42.md"
        );
        let matrix: UserKey = "matrix:user:%40alice%3Aexample.org".parse().unwrap();
        assert_eq!(
            expect(Kind::People.key(matrix.as_str())),
            keys::person_record(&matrix)
        );
        assert_eq!(expect(Kind::Events.key("2026-01")), "olaylar/2026-01.md");
        assert!(Kind::parse("INDEX.md").is_err());
        assert!(Kind::parse("temperament").is_err());
    }

    #[test]
    fn person_unknown_content_and_favorite_constraints_are_explicit() {
        let id = UserKey::discord(42);
        let person = Person {
            id: id.clone(),
            name: "Ada".into(),
            score: 10,
            note: store::FAVORITE_NOTE.into(),
            ..Default::default()
        };
        let text = person.text();
        assert!(structured_safe(&id, &text));
        for raw in [
            format!("{text}\nunknown: retained knowledge\n"),
            format!("{text}\n## Unrecognized section\n- retained fact\n"),
            format!("{text}\nImportant prose that is not a bullet.\n"),
        ] {
            assert!(!structured_safe(&id, &raw));
        }
        let input = PersonInput::from_person(&person);
        assert!(input.clone().text(&id, true).is_ok());
        let mut changed = input.clone();
        changed.score = 9;
        assert!(changed.text(&id, true).is_err());
        let mut changed = input.clone();
        changed.note = "overridden favorite note".into();
        assert!(changed.text(&id, true).is_err());
        let mut changed = input.clone();
        changed.score = -11;
        assert!(changed.text(&id, false).is_err());
        let mut changed = input.clone();
        changed.tags = (0..7).map(|index| format!("tag{index}")).collect();
        assert!(changed.text(&id, false).is_err());
        let mut changed = input.clone();
        changed.previous_names = (0..6).map(|index| format!("name{index}")).collect();
        assert!(changed.text(&id, false).is_err());
        let mut changed = input.clone();
        changed.tags = (0..6).map(|index| format!("tag{index}")).collect();
        changed.previous_names = (0..5).map(|index| format!("name{index}")).collect();
        assert!(changed.text(&id, false).is_ok());
        let mut changed = input;
        changed.name = "Ada\nid: discord:user:43".into();
        assert!(changed.text(&id, false).is_err());
    }

    #[test]
    fn query_and_content_bounds_and_strong_preconditions_are_checked() {
        assert!(browse_options(&HashMap::from([("query".into(), "x".repeat(257))])).is_err());
        assert!(browse_options(&HashMap::from([("limit".into(), "101".into())])).is_err());
        assert!(browse_options(&HashMap::from([("limit".into(), "0".into())])).is_err());
        assert!(browse_options(&HashMap::from([("cursor".into(), "x".repeat(2049))])).is_err());
        assert!(validate_content(&"x".repeat(MAX_CONTENT + 1)).is_err());
        assert!(validate_content("embedded\0null").is_err());
        for invalid in ["*", "0", "W/\"0\"", "\"0\",\"1\"", "\"-1\""] {
            let mut headers = HeaderMap::new();
            headers.insert(header::IF_MATCH, HeaderValue::from_str(invalid).unwrap());
            assert!(precondition(&headers).is_err());
        }
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("\"0\""));
        assert_eq!(expect(precondition(&headers)), 0);
    }

    #[tokio::test]
    async fn routes_enforce_auth_csrf_origin_bounds_and_compare_and_swap() {
        let (app, _directory) = super::super::tests::fixture().await;
        let initial = seed(
            "konular/panel-memory-cas-test.md",
            "# Initial\nFull prose.\n",
        );
        let router = super::super::routes::router(app.clone());
        for path in [
            "/memory",
            "/memory/topics/panel-memory-cas-test",
            "/api/v1/memory/topics",
            "/api/v1/memory/topics/panel-memory-cas-test",
        ] {
            let response = router
                .clone()
                .oneshot(super::super::tests::request("GET", path, None, ""))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        }
        let unauthorized = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/memory/topics/panel-memory-cas-test",
                None,
                "revision=0&content=draft",
            ))
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let session = super::super::tests::owner(&app).await;
        let path = "/api/v1/memory/topics/panel-memory-cas-test";
        let mut unauthorized_put = put(
            path,
            &session,
            Some(initial.revision),
            true,
            json!({"content":"draft"}),
        );
        unauthorized_put.headers_mut().remove(header::COOKIE);
        assert_eq!(
            router
                .clone()
                .oneshot(unauthorized_put)
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let no_csrf = router
            .clone()
            .oneshot(put(
                path,
                &session,
                Some(initial.revision),
                false,
                json!({"content":"draft"}),
            ))
            .await
            .unwrap();
        assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);
        let absent = router
            .clone()
            .oneshot(put(path, &session, None, true, json!({"content":"draft"})))
            .await
            .unwrap();
        assert_eq!(absent.status(), StatusCode::PRECONDITION_REQUIRED);
        assert_eq!(json_body(absent).await["code"], "revision_required");
        let mut cross_origin = put(
            path,
            &session,
            Some(initial.revision),
            true,
            json!({"content":"draft"}),
        );
        cross_origin.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("http://attacker.invalid"),
        );
        assert_eq!(
            router.clone().oneshot(cross_origin).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let saved = router
            .clone()
            .oneshot(put(
                path,
                &session,
                Some(initial.revision),
                true,
                json!({"content":"# Saved\nAll prose.\n"}),
            ))
            .await
            .unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let new_etag = saved.headers()[header::ETAG].clone();
        let saved = json_body(saved).await;
        assert_eq!(
            saved["record"]["revision"].as_u64().unwrap(),
            initial.revision + 1
        );
        let stale = router
            .clone()
            .oneshot(put(
                path,
                &session,
                Some(initial.revision),
                true,
                json!({"content":"MY UNSAVED DRAFT"}),
            ))
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        assert_eq!(stale.headers()[header::ETAG], new_etag);
        let conflict = json_body(stale).await;
        assert_eq!(conflict["code"], "revision_conflict");
        assert_eq!(conflict["current"]["content"], "# Saved\nAll prose.\n");
        assert_eq!(conflict["submitted_draft"], "MY UNSAVED DRAFT");
        assert!(conflict["request_id"].as_str().is_some());
        let get = router
            .clone()
            .oneshot(super::super::tests::request(
                "GET",
                path,
                Some(&session.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(get.headers()[header::ETAG], new_etag);
        assert_eq!(json_body(get).await["content"], "# Saved\nAll prose.\n");
        let oversized = router
            .clone()
            .oneshot(put(
                path,
                &session,
                Some(initial.revision + 1),
                true,
                json!({"content":"x".repeat(33*1024)}),
            ))
            .await
            .unwrap();
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let missing = router
            .clone()
            .oneshot(put(
                "/api/v1/memory/topics/panel-memory-missing-cas-test",
                &session,
                Some(0),
                true,
                json!({"content":"draft"}),
            ))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        for path in [
            "/api/v1/memory/topics/..%2Fhuy",
            "/api/v1/memory/people/42",
            "/api/v1/memory/events/2026-13",
            "/api/v1/memory/INDEX.md",
        ] {
            let response = router
                .clone()
                .oneshot(super::super::tests::request(
                    "GET",
                    path,
                    Some(&session.token),
                    "",
                ))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{path}"
            );
        }
        let body = format!(
            "csrf={}&revision={}&content=FORM-DRAFT&lang=en",
            session.info.csrf, initial.revision
        );
        let stale_form = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/memory/topics/panel-memory-cas-test",
                Some(&session.token),
                &body,
            ))
            .await
            .unwrap();
        assert_eq!(stale_form.status(), StatusCode::PRECONDITION_FAILED);
        let html = String::from_utf8(
            to_bytes(stale_form.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("FORM-DRAFT"));
        assert!(html.contains("# Saved"));
        assert!(html.contains("readonly"));
        assert!(html.contains("/memory/topics/panel-memory-cas-test"));
        let no_csrf_form = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/memory/topics/panel-memory-cas-test",
                Some(&session.token),
                "revision=0&content=draft",
            ))
            .await
            .unwrap();
        assert_eq!(no_csrf_form.status(), StatusCode::FORBIDDEN);
        let no_revision_form = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/memory/topics/panel-memory-cas-test",
                Some(&session.token),
                &format!("csrf={}&content=draft", session.info.csrf),
            ))
            .await
            .unwrap();
        assert_eq!(no_revision_form.status(), StatusCode::PRECONDITION_REQUIRED);
        let oversized_form = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/memory/topics/panel-memory-cas-test",
                Some(&session.token),
                &format!(
                    "csrf={}&revision={}&content={}",
                    session.info.csrf,
                    initial.revision + 1,
                    "x".repeat(97 * 1024)
                ),
            ))
            .await
            .unwrap();
        assert_eq!(oversized_form.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let form_success = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/memory/topics/panel-memory-cas-test",
                Some(&session.token),
                &format!(
                    "csrf={}&revision={}&content=FORM-SAVED",
                    session.info.csrf,
                    initial.revision + 1
                ),
            ))
            .await
            .unwrap();
        assert_eq!(form_success.status(), StatusCode::SEE_OTHER);
        let location = form_success.headers()[header::LOCATION].to_str().unwrap();
        assert!(location.starts_with("/memory/topics/panel-memory-cas-test?"));
        assert!(location.contains("saved=1"));
        let reloaded = router
            .oneshot(super::super::tests::request(
                "GET",
                location,
                Some(&session.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(reloaded.status(), StatusCode::OK);
        assert_eq!(
            store::read_versioned("konular/panel-memory-cas-test.md")
                .unwrap()
                .unwrap()
                .content,
            "FORM-SAVED"
        );
    }

    #[tokio::test]
    async fn person_unknown_fields_are_not_overwritten_and_event_summary_is_visible() {
        let (app, _directory) = super::super::tests::fixture().await;
        let session = super::super::tests::owner(&app).await;
        let identity = UserKey::discord(8_800_000_000_000_000_351);
        let person = Person {
            id: identity.clone(),
            name: "Ada".into(),
            ..Default::default()
        };
        let original = format!("{}\nextra_field: MUST REMAIN\n", person.text());
        let seeded = seed(&keys::person_record(&identity), &original);
        let router = super::super::routes::router(app.clone());
        let path = format!(
            "/api/v1/memory/people/{}",
            encode_component(identity.as_str())
        );
        let response = router
            .clone()
            .oneshot(put(
                &path,
                &session,
                Some(seeded.revision),
                true,
                json!({"person":PersonInput::from_person(&person)}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json_body(response).await["code"], "unsafe_person_format");
        assert_eq!(
            store::read_versioned(&keys::person_record(&identity))
                .unwrap()
                .unwrap()
                .content,
            original
        );
        let get = router
            .clone()
            .oneshot(super::super::tests::request(
                "GET",
                &path,
                Some(&session.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(json_body(get).await["structured_editable"], false);
        seed(
            "olaylar/1801-02.md",
            "This is summary prose, not a bullet.\n\n- 1801-02-03 #room: RAW EVENT LINE\n",
        );
        let get = router
            .clone()
            .oneshot(super::super::tests::request(
                "GET",
                "/memory/events/1801-02",
                Some(&session.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::OK);
        let html = String::from_utf8(
            to_bytes(get.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("This is summary prose, not a bullet."));
        assert!(html.contains("RAW EVENT LINE"));
        assert!(html.contains("<textarea"));
        let get = router
            .oneshot(super::super::tests::request(
                "GET",
                "/api/v1/memory/events/1801-02",
                Some(&session.token),
                "",
            ))
            .await
            .unwrap();
        let value = json_body(get).await;
        assert!(value["content"].as_str().unwrap().contains("summary prose"));
        assert!(value["content"]
            .as_str()
            .unwrap()
            .contains("RAW EVENT LINE"));
    }

    #[tokio::test]
    async fn full_search_pagination_exceeds_old_topic_limit_without_duplicates() {
        let (app, _directory) = super::super::tests::fixture().await;
        let session = super::super::tests::owner(&app).await;
        for index in 0..45 {
            seed(
                &format!("konular/panel-memory-pagination-{index:02}.md"),
                &format!(
                    "# Topic {index}\nUniquePanelPaginationNeedle\n{}",
                    "full prose ".repeat(100)
                ),
            );
        }
        let router = super::super::routes::router(app);
        let mut cursor = None;
        let mut identities = std::collections::HashSet::new();
        loop {
            let suffix = cursor
                .as_ref()
                .map(|cursor: &String| format!("&cursor={}", encode_component(cursor)))
                .unwrap_or_default();
            let path =
                format!("/api/v1/memory/topics?query=UniquePanelPaginationNeedle&limit=7{suffix}");
            let response = router
                .clone()
                .oneshot(super::super::tests::request(
                    "GET",
                    &path,
                    Some(&session.token),
                    "",
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let value = json_body(response).await;
            let records = value["records"].as_array().unwrap();
            assert!(records.len() <= 7);
            for record in records {
                assert!(identities.insert(record["id"].as_str().unwrap().to_owned()));
                assert!(record["content"].as_str().unwrap().len() > 1000);
            }
            cursor = value["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(identities.len(), 45);
    }

    #[tokio::test]
    async fn favorite_score_and_identity_are_locked_in_api_and_native_form() {
        let (app, _directory) = super::super::tests::fixture().await;
        let id = UserKey::discord(8_800_000_000_000_000_352);
        let config_path = app.config.lock().path().to_owned();
        let configured = crate::config::Config::load(
            config_path,
            &HashMap::from([("OWNER_ID".into(), "8800000000000000352".into())]),
        )
        .unwrap();
        *app.config.lock() = configured;
        let session = super::super::tests::owner(&app).await;
        let person = Person {
            id: id.clone(),
            name: "Favorite".into(),
            score: 10,
            note: store::FAVORITE_NOTE.into(),
            ..Default::default()
        };
        let initial = seed(&keys::person_record(&id), &person.text());
        let router = super::super::routes::router(app);
        let path = format!("/api/v1/memory/people/{}", encode_component(id.as_str()));
        let mut changed = PersonInput::from_person(&person);
        changed.score = 9;
        let rejected = router
            .clone()
            .oneshot(put(
                &path,
                &session,
                Some(initial.revision),
                true,
                json!({"person":changed}),
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let mut changed = PersonInput::from_person(&person);
        changed.name = "Favorite corrected name".into();
        let accepted = router
            .clone()
            .oneshot(put(
                &path,
                &session,
                Some(initial.revision),
                true,
                json!({"person":changed}),
            ))
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
        let accepted = json_body(accepted).await;
        assert_eq!(accepted["record"]["person"]["score"], 10);
        assert_eq!(accepted["record"]["id"], id.as_str());
        let revision = accepted["record"]["revision"].as_u64().unwrap();
        let immutable = format!(
            "csrf={}&revision={revision}&identity=discord%3Auser%3A43&name=Favorite&score=10",
            session.info.csrf
        );
        let form_path = format!("/ui/memory/people/{}", encode_component(id.as_str()));
        let rejected = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                &form_path,
                Some(&session.token),
                &immutable,
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let mut injected = serde_json::to_value(PersonInput::from_person(&person)).unwrap();
        injected["id"] = json!("discord:user:43");
        let rejected = router
            .oneshot(put(
                &path,
                &session,
                Some(revision),
                true,
                json!({"person":injected}),
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let current = store::read_versioned(&keys::person_record(&id))
            .unwrap()
            .unwrap();
        assert_eq!(current.revision, revision);
        assert_eq!(Person::parse(&id, &current.content).score, 10);
    }

    #[tokio::test]
    async fn native_unicode_document_round_trips_with_redirect_confirmation() {
        let (app, _directory) = super::super::tests::fixture().await;
        let session = super::super::tests::owner(&app).await;
        let initial = seed("konular/panel-memory-unicode-form.md", "# Original\n");
        let text = "ş".repeat(6000);
        let encoded = encode_component(&text);
        let body = format!(
            "csrf={}&revision={}&content={encoded}&lang=tr",
            session.info.csrf, initial.revision
        );
        assert!(body.len() > 32 * 1024);
        assert!(text.len() < MAX_CONTENT);
        let router = super::super::routes::router(app);
        let response = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/memory/topics/panel-memory-unicode-form?lang=tr",
                Some(&session.token),
                &body,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let location = response.headers()[header::LOCATION].to_str().unwrap();
        assert!(location.contains("lang=tr"));
        assert!(location.contains("saved=1"));
        let response = router
            .oneshot(super::super::tests::request(
                "GET",
                location,
                Some(&session.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(
            to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains(&text));
        assert!(html.contains(&t(Language::Tr, "panel.memory.saved")));
        assert_eq!(
            store::read_versioned("konular/panel-memory-unicode-form.md")
                .unwrap()
                .unwrap()
                .content,
            text
        );
    }

    #[tokio::test]
    async fn memory_mutation_rejects_runtime_starting_or_stopping_inside_worker() {
        let (app, _directory) = super::super::tests::fixture().await;
        let initial = seed("konular/panel-memory-runtime-transition.md", "# Original\n");
        for lifecycle in [
            super::super::Lifecycle::ConfiguredStarting,
            super::super::Lifecycle::Stopping,
        ] {
            app.runtime.lock().lifecycle = lifecycle;
            let result = replace(
                &app,
                Kind::Topics,
                "panel-memory-runtime-transition".into(),
                initial.revision,
                SaveInput {
                    csrf: String::new(),
                    content: Some("must not save".into()),
                    person: None,
                },
            )
            .await;
            assert!(result.is_err());
            let current = store::read_versioned("konular/panel-memory-runtime-transition.md")
                .unwrap()
                .unwrap();
            assert_eq!(current.revision, initial.revision);
            assert_eq!(current.content, initial.content);
        }
    }

    #[tokio::test]
    async fn migrated_discord_numeric_body_is_editable_without_identity_aliases() {
        let (app, _directory) = super::super::tests::fixture().await;
        let session = super::super::tests::owner(&app).await;
        let id = UserKey::discord(8_800_000_000_000_000_353);
        let person = Person {
            id: id.clone(),
            name: "Migrated person".into(),
            score: 3,
            facts: vec!["Existing fact stays remembered".into()],
            ..Default::default()
        };
        let original =
            person
                .text()
                .replacen(&format!("id: {id}\n"), "id: 8800000000000000353\n", 1);
        assert!(structured_safe(&id, &original));
        assert!(!structured_safe(
            &id,
            &original.replace("id: 8800000000000000353\n", "id: 8800000000000000354\n",)
        ));
        assert!(!structured_safe(
            &id,
            &format!("{original}unknown: retained\n")
        ));
        let telegram: UserKey = "telegram:user:8800000000000000353".parse().unwrap();
        assert!(!structured_safe(&telegram, &original));
        let initial = seed(&keys::person_record(&id), &original);
        let router = super::super::routes::router(app);
        let path = format!("/api/v1/memory/people/{}", encode_component(id.as_str()));
        let response = router
            .clone()
            .oneshot(super::super::tests::request(
                "GET",
                &path,
                Some(&session.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["structured_editable"], true);
        let mut changed = PersonInput::from_person(&person);
        changed.note = "Corrected without losing migrated facts".into();
        let response = router
            .oneshot(put(
                &path,
                &session,
                Some(initial.revision),
                true,
                json!({"person": changed}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let saved = json_body(response).await;
        assert_eq!(saved["record"]["id"], id.as_str());
        assert!(saved["record"]["content"]
            .as_str()
            .unwrap()
            .contains(&format!("id: {id}\n")));
        assert!(saved["record"]["content"]
            .as_str()
            .unwrap()
            .contains("Existing fact stays remembered"));
        let previous =
            store::test::historical_snapshot(&keys::person_record(&id), initial.revision);
        assert_eq!(previous.key, keys::person_record(&id));
        assert_eq!(previous.content, original);
        assert_eq!(previous.revision, initial.revision);
    }
}

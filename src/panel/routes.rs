use super::{
    actions, auth, dashboard, memory, personality, settings, setup, usage, views, App, Lifecycle,
};
use crate::config::{Patch, PatchValue};
use crate::platform::types::PlatformKind;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response, Sse};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use views::{Button, Field, Form as ViewForm, Item, Language, Page, Section};

pub(super) type WebResult<T> = Result<T, WebError>;
#[derive(Clone, Debug)]
pub(super) struct WebError {
    status: StatusCode,
    code: String,
    key: String,
    fields: HashMap<String, String>,
}

/// Fully localized HTML errors (including editable conflict drafts) keep their body.
#[derive(Clone, Copy, Debug)]
pub(super) struct PreserveErrorBody;

/// Safe structured error metadata, serialized once with the middleware's request ID.
#[derive(Clone, Debug)]
pub(super) struct ErrorDetails(pub Value);
impl WebError {
    pub(super) fn new(status: StatusCode, code: &str, key: &str) -> Self {
        Self {
            status,
            code: code.into(),
            key: key.into(),
            fields: HashMap::new(),
        }
    }
    pub(super) fn validation() -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "panel.error.validation",
        )
    }

    pub(super) fn config(error: crate::config::ConfigError) -> Self {
        use crate::config::ConfigError;
        let status = match &error {
            ConfigError::Field { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            ConfigError::Conflict | ConfigError::AlreadyActive => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        let mut result = Self::new(status, error.code(), error.code());
        if let ConfigError::Field { key, code } = error {
            result.fields.insert(key, code.into());
        }
        result
    }
}
impl From<auth::AuthError> for WebError {
    fn from(error: auth::AuthError) -> Self {
        let status = match error {
            auth::AuthError::Required | auth::AuthError::InvalidCredentials => {
                StatusCode::UNAUTHORIZED
            }
            auth::AuthError::RateLimited
            | auth::AuthError::Busy
            | auth::AuthError::SessionLimit => StatusCode::TOO_MANY_REQUESTS,
            auth::AuthError::Storage(_) | auth::AuthError::Internal | auth::AuthError::Random => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            _ => StatusCode::FORBIDDEN,
        };
        Self::new(status, error.code(), error.message_key())
    }
}
impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let message = views::text(Language::En, &self.key).unwrap_or(&self.code);
        let mut response = (self.status, Json(json!({"code":self.code,"message":message,"field_errors":self.fields,"request_id":request_id()}))).into_response();
        response.extensions_mut().insert(self);
        response
    }
}
fn request_id() -> String {
    use rand::RngCore;
    let mut bytes = [0; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(super) fn t(lang: Language, key: &str) -> String {
    views::text(lang, key).unwrap_or(key).to_owned()
}
pub(super) fn lang(query: &HashMap<String, String>) -> Language {
    Language::parse(query.get("lang").map_or("en", String::as_str))
}
pub(super) fn token(headers: &HeaderMap) -> WebResult<String> {
    auth::cookie_token(headers)?.ok_or_else(|| auth::AuthError::Required.into())
}
pub(super) fn admin(app: &App, headers: &HeaderMap) -> WebResult<String> {
    let token = token(headers)?;
    app.auth.require_admin(&token)?;
    Ok(token)
}
pub(super) fn mutation(app: &App, headers: &HeaderMap, csrf: &str) -> WebResult<String> {
    let token = admin(app, headers)?;
    app.auth.mutation(&token, csrf, auth::SessionScope::Admin)?;
    if *app.stop.borrow() {
        return Err(WebError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "stopping",
            "panel.state.stopping",
        ));
    }
    Ok(token)
}

/// Use inside a synchronous editor worker, never across `.await`.
/// Holding this lifecycle guard prevents setup from loading caches between an
/// offline edit's Bot selection and its commit/publication.
pub(super) fn edit_runtime(app: &App) -> WebResult<parking_lot::MutexGuard<'_, super::Runtime>> {
    let runtime = app.runtime.lock();
    if runtime.lifecycle == Lifecycle::ConfiguredStarting {
        return Err(WebError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_starting",
            "panel.memory.runtime_starting",
        ));
    }
    if runtime.lifecycle == Lifecycle::Stopping || *app.stop.borrow() {
        return Err(WebError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "stopping",
            "panel.state.stopping",
        ));
    }
    Ok(runtime)
}
fn draft_key(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
pub(super) fn html(page: &Page) -> WebResult<Response> {
    views::render(page)
        .map(|body| Html(body).into_response())
        .map_err(|_| {
            WebError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "render_failed",
                "panel.auth_internal",
            )
        })
}
fn cookie_response(
    mut response: Response,
    issue: &auth::SessionIssue,
    secure: bool,
) -> WebResult<Response> {
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&issue.cookie(secure)).map_err(|_| WebError::validation())?,
    );
    Ok(response)
}
pub(super) fn button(lang: Language, key: &str) -> Button {
    Button {
        label: t(lang, key),
        ..Default::default()
    }
}
pub(super) fn field(lang: Language, name: &str, key: &str, kind: &str, value: &str) -> Field {
    Field {
        name: name.into(),
        label: t(lang, key),
        kind: kind.into(),
        value: value.into(),
        ..Default::default()
    }
}
pub(super) fn form(
    id: &str,
    action: &str,
    csrf: &str,
    fields: Vec<Field>,
    buttons: Vec<Button>,
) -> ViewForm {
    ViewForm {
        id: id.into(),
        action: action.into(),
        csrf: csrf.into(),
        fields,
        buttons,
        ..Default::default()
    }
}
pub(super) fn section(lang: Language, key: &str) -> Section {
    Section {
        title: t(lang, key),
        ..Default::default()
    }
}
pub(super) fn item(lang: Language, label: &str, value: impl ToString, key: &str) -> Item {
    Item {
        label: t(lang, label),
        value: value.to_string(),
        key: key.into(),
        ..Default::default()
    }
}
pub(super) fn page(lang: Language, title: &str, active: &str) -> WebResult<Page> {
    Page::new(lang, title, active).map_err(|_| WebError::validation())
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
        .route(
            "/assets/panel.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    views::CSS,
                )
            }),
        )
        .route(
            "/assets/panel.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    views::JS,
                )
            }),
        )
        .route(
            "/assets/htmx-2.0.4.min.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    views::HTMX_JS,
                )
            }),
        )
        .route("/", get(dashboard::overview))
        .route("/setup", get(setup_page))
        .route("/setup/slack-app-manifest.yml", get(slack_manifest))
        .route("/login", get(login_page))
        .route("/settings", get(settings::view))
        .route("/logs", get(logs_page))
        .route("/account", get(account_page))
        .route("/help", get(help_page))
        .route("/ui/setup/claim", post(ui_claim))
        .route("/ui/setup/protect", post(ui_protect))
        .route("/ui/setup/platform", post(ui_platform))
        .route("/ui/setup/provider", post(ui_provider))
        .route("/ui/setup/fit", post(ui_fit))
        .route("/ui/setup/complete", post(ui_complete))
        .route("/ui/auth/login", post(ui_login))
        .route("/ui/auth/logout", post(ui_logout))
        .route("/ui/auth/password", post(ui_password))
        .route("/ui/settings", post(settings::ui))
        .route("/ui/settings/test-platform", post(settings::platform_ui))
        .route("/ui/settings/test-provider", post(settings::provider_ui))
        .route("/api/v1/setup/claim", post(api_claim))
        .route("/api/v1/setup/status", get(setup_status))
        .route("/api/v1/setup/test-platform", post(api_platform))
        .route("/api/v1/setup/test-provider", post(api_provider))
        .route("/api/v1/setup/complete", post(api_complete))
        .route("/api/v1/auth/login", post(api_login))
        .route("/api/v1/auth/logout", post(api_logout))
        .route("/api/v1/auth/password", post(api_password))
        .route("/api/v1/auth/session", get(auth_session))
        .route("/api/v1/status", get(dashboard::status_api))
        .route(
            "/api/v1/settings",
            get(settings::api).patch(settings::patch),
        )
        .route("/api/v1/settings/validate", post(settings::validate))
        .route(
            "/api/v1/settings/test-platform",
            post(settings::platform_api),
        )
        .route(
            "/api/v1/settings/test-provider",
            post(settings::provider_api),
        )
        .route("/api/v1/logs", get(logs_api))
        .route("/api/v1/logs/stream", get(log_stream))
        .merge(memory::router())
        .merge(personality::router())
        .merge(actions::router())
        .merge(usage::router())
        .layer(DefaultBodyLimit::max(32 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), security))
        .with_state(app)
}

async fn security(
    State(app): State<Arc<App>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let write = !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    );
    let path = request.uri().path().to_owned();
    let language = Language::parse(
        request
            .uri()
            .query()
            .unwrap_or("")
            .split('&')
            .find_map(|part| part.strip_prefix("lang="))
            .unwrap_or("en"),
    );
    let identifier = request_id();
    let mut response = match app.origin.check(request.headers(), write) {
        Ok(()) => next.run(request).await,
        Err(error) => WebError::from(error).into_response(),
    };
    if let Some(ErrorDetails(mut details)) = response.extensions_mut().remove::<ErrorDetails>() {
        details["request_id"] = json!(identifier);
        let headers = std::mem::take(response.headers_mut());
        response = (response.status(), Json(details)).into_response();
        response.headers_mut().extend(headers);
    } else if (response.status().is_client_error() || response.status().is_server_error())
        && response.extensions().get::<PreserveErrorBody>().is_none()
    {
        let status = response.status();
        let error = response
            .extensions()
            .get::<WebError>()
            .cloned()
            .unwrap_or_else(WebError::validation);
        let key = error.key.as_str();
        if path.starts_with("/api/") {
            let fields: HashMap<_, _> = error
                .fields
                .iter()
                .map(|(field, key)| (field, t(language, key)))
                .collect();
            response = (status, Json(json!({"code":error.code,"message":t(language,key),"field_errors":fields,"request_id":identifier}))).into_response();
        } else {
            let (title, back) =
                if status == StatusCode::UNAUTHORIZED || path.contains("/auth/login") {
                    ("panel.title.login", "/login")
                } else if path.contains("/settings") {
                    ("panel.title.settings", "/settings")
                } else if path.contains("/setup") {
                    ("panel.title.setup", "/setup")
                } else if path.contains("/memory") {
                    ("panel.title.memory", "/memory")
                } else if path.contains("/personality") {
                    ("panel.title.personality", "/personality")
                } else if path.contains("/actions") || path.contains("/jobs") {
                    ("panel.actions.title", "/actions")
                } else if path.contains("/usage") {
                    ("panel.title.usage", "/usage")
                } else {
                    ("panel.title.account", "/account")
                };
            if let Ok(mut error_page) = Page::new(language, title, "") {
                error_page.error = t(language, key);
                for (field, key) in &error.fields {
                    error_page.error.push_str(&format!(
                        "\n{}: {}",
                        t(language, &format!("panel.settings.{field}.label")),
                        t(language, key)
                    ));
                }
                error_page.sections.push(Section {
                    items: vec![Item {
                        value: t(language, "panel.action.back"),
                        href: format!("{back}?lang={}", language.code()),
                        ..Default::default()
                    }],
                    ..Default::default()
                });
                if let Ok(body) = views::render(&error_page) {
                    response = (status, Html(body)).into_response();
                }
            }
        }
    }
    if path.starts_with("/ui/") && language == Language::Tr && response.status().is_redirection() {
        if let Some(location) = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.starts_with('/') && !value.contains("lang="))
        {
            let location = format!(
                "{location}{}lang=tr",
                if location.contains('?') { "&" } else { "?" }
            );
            if let Ok(location) = HeaderValue::from_str(&location) {
                response.headers_mut().insert(header::LOCATION, location);
            }
        }
    }
    for (name,value) in [
        ("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'; form-action 'self'"),
        ("x-content-type-options","nosniff"),("referrer-policy","same-origin"),("cache-control","no-store"),("x-frame-options","DENY")
    ] { response.headers_mut().insert(name,HeaderValue::from_static(value)); }
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&identifier).expect("hex request ID"),
    );
    response
}

async fn public_session(
    app: &App,
    headers: &HeaderMap,
    peer: SocketAddr,
) -> WebResult<(String, auth::SessionInfo, Option<auth::SessionIssue>)> {
    if let Some(token) = auth::cookie_token(headers)? {
        if let Ok(info) = app.auth.session(&token) {
            return Ok((token, info, None));
        }
    }
    let issue = app.auth.preauth(peer.ip())?;
    Ok((issue.token.clone(), issue.info.clone(), Some(issue)))
}

async fn login_page(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    if !app.auth.has_admin()? {
        return Ok(Redirect::to("/setup").into_response());
    }
    let (_, info, issue) = public_session(&app, &headers, peer).await?;
    if info.scope == auth::SessionScope::Admin {
        return Ok(Redirect::to("/").into_response());
    }
    let lang = lang(&query);
    let mut page = page(lang, "panel.title.login", "")?;
    page.description = t(lang, "panel.login.body");
    let mut card = section(lang, "panel.title.login");
    let mut password = field(lang, "password", "panel.label.password", "password", "");
    password.required = true;
    password.autocomplete = "current-password".into();
    card.forms.push(form(
        "login-form",
        "/ui/auth/login",
        &info.csrf,
        vec![password],
        vec![button(lang, "panel.action.login")],
    ));
    page.sections.push(card);
    let response = html(&page)?;
    match issue {
        Some(issue) => cookie_response(response, &issue, app.secure),
        None => Ok(response),
    }
}

pub(super) fn platform_credentials(platform: PlatformKind) -> &'static [&'static str] {
    match platform {
        PlatformKind::Discord => &["DISCORD_TOKEN"],
        PlatformKind::Telegram => &["TELEGRAM_TOKEN"],
        PlatformKind::Slack => &["SLACK_APP_TOKEN", "SLACK_BOT_TOKEN"],
        PlatformKind::Matrix => &["MATRIX_HOMESERVER", "MATRIX_USER", "MATRIX_ACCESS_TOKEN"],
        PlatformKind::Terminal => &[],
    }
}

pub(super) fn fit_fields(platform: PlatformKind) -> &'static [&'static str] {
    match platform {
        PlatformKind::Discord => &[
            "BOT_LANG",
            "TZ_OFFSET",
            "OWNER_KEY",
            "GUILD_ID",
            "CHANNELS",
            "NEWS_CHANNEL",
            "NEWS_RSS",
        ],
        PlatformKind::Telegram => &[
            "BOT_LANG",
            "TZ_OFFSET",
            "OWNER_KEY",
            "TELEGRAM_ROOMS",
            "NEWS_RSS",
        ],
        PlatformKind::Slack => &[
            "BOT_LANG",
            "TZ_OFFSET",
            "OWNER_KEY",
            "SLACK_CHANNELS",
            "NEWS_RSS",
        ],
        PlatformKind::Matrix => &[
            "BOT_LANG",
            "TZ_OFFSET",
            "OWNER_KEY",
            "MATRIX_ROOMS",
            "NEWS_RSS",
        ],
        PlatformKind::Terminal => &[],
    }
}

pub(super) fn select_platform(draft: &mut setup::Draft, platform: PlatformKind) {
    if draft.selected_platform != platform {
        draft.selected_platform = platform;
        draft.platform_attempt += 1;
        draft.application = None;
        draft.fit_completed = false;
    }
}

pub(super) fn reserve_platform(
    draft: &mut setup::Draft,
    platform: PlatformKind,
) -> ([u8; 16], u64) {
    select_platform(draft, platform);
    draft.platform_attempt += 1;
    draft.application = None;
    draft.fit_completed = false;
    (draft.platform_nonce, draft.platform_attempt)
}

pub(super) fn publish_platform(
    draft: &mut setup::Draft,
    attempt: ([u8; 16], u64),
    application: setup::Application,
    credentials: HashMap<String, String>,
) -> WebResult<()> {
    if draft.expires <= Instant::now()
        || (draft.platform_nonce, draft.platform_attempt) != attempt
        || draft.selected_platform != application.platform
    {
        return Err(WebError::new(
            StatusCode::CONFLICT,
            "stale_platform_test",
            "panel.setup.stale_test",
        ));
    }
    draft.values.extend(credentials);
    draft.application = Some(application);
    draft.expires = Instant::now() + std::time::Duration::from_secs(900);
    Ok(())
}

async fn slack_manifest(State(app): State<Arc<App>>, headers: HeaderMap) -> WebResult<Response> {
    admin(&app, &headers)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/yaml; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"slack-app-manifest.yml\"",
            ),
        ],
        include_str!("../../docs/slack-app-manifest.yml"),
    )
        .into_response())
}

async fn setup_page(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let (token, info, issue) = public_session(&app, &headers, peer).await?;
    if app.auth.has_admin()? && info.scope != auth::SessionScope::Admin {
        return Ok(Redirect::to("/login").into_response());
    }
    let lang = lang(&query);
    let requested_platform = query
        .get("platform")
        .map(|platform| setup::parse_platform(platform))
        .transpose()
        .map_err(|_| WebError::validation())?;
    let mut page = page(lang, "panel.title.setup", "")?;
    page.authenticated = info.scope == auth::SessionScope::Admin;
    let mut step = query
        .get("step")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 5);
    if info.scope != auth::SessionScope::Admin {
        step = 1;
    }
    if info.scope == auth::SessionScope::Admin && step == 1 {
        step = 2;
    }
    if info.scope == auth::SessionScope::Admin {
        settings::refresh(app.clone()).await?;
        let config = app.config.lock();
        let mut drafts = app.drafts.lock();
        drafts.retain(|_, draft| draft.expires > Instant::now());
        let draft = drafts
            .entry(draft_key(&token))
            .or_insert_with(|| setup::Draft::new(&config));
        if requested_platform.is_some() {
            // Choosing a card only renders its form. CSRF-protected POST owns draft changes.
            step = 2;
        }
        step = step.min(draft.step());
    }
    page.setup_steps(step).map_err(|_| WebError::validation())?;
    let mut card = section(lang, &format!("panel.setup.step{step}"));
    if info.scope == auth::SessionScope::Preauth {
        card.description = t(lang, "panel.setup.claim");
        let mut code = field(lang, "token", "panel.label.token", "password", "");
        code.required = true;
        code.autocomplete = "off".into();
        card.forms.push(form(
            "claim-form",
            "/ui/setup/claim",
            &info.csrf,
            vec![code],
            vec![button(lang, "panel.action.continue")],
        ));
    } else if info.scope == auth::SessionScope::Setup {
        app.auth.require_setup(&token)?;
        card.description = t(lang, "panel.setup.protect");
        let mut password = field(lang, "password", "panel.label.new_password", "password", "");
        password.required = true;
        password.autocomplete = "new-password".into();
        card.forms.push(form(
            "protect-form",
            "/ui/setup/protect",
            &info.csrf,
            vec![password],
            vec![button(lang, "panel.action.continue")],
        ));
    } else {
        let (values, selected_platform, application) = {
            let drafts = app.drafts.lock();
            let draft = drafts
                .get(&draft_key(&token))
                .ok_or_else(WebError::validation)?;
            (
                draft.values.clone(),
                requested_platform.unwrap_or(draft.selected_platform),
                draft.application.clone(),
            )
        };
        let get = |key: &str| values.get(key).map_or("", String::as_str);
        let config_fields = app.config.lock().fields();
        match step {
            2 => {
                card.description = t(lang, "panel.setup.platform");
                card.platform_cards = [
                    PlatformKind::Telegram,
                    PlatformKind::Discord,
                    PlatformKind::Slack,
                    PlatformKind::Matrix,
                ]
                .into_iter()
                .map(|platform| views::PlatformCard {
                    name: platform.as_str(),
                    title: t(lang, &format!("panel.setup.{}.name", platform.as_str())),
                    description: t(lang, &format!("panel.setup.{}.card", platform.as_str())),
                    badge: if platform == PlatformKind::Telegram {
                        t(lang, "panel.setup.recommended")
                    } else {
                        String::new()
                    },
                    href: format!(
                        "/setup?step=2&platform={}&lang={}",
                        platform.as_str(),
                        lang.code()
                    ),
                    selected: platform == selected_platform,
                })
                .collect();
                card.items.push(item(
                    lang,
                    "panel.setup.instructions",
                    t(
                        lang,
                        &format!("panel.setup.{}.credentials", selected_platform.as_str()),
                    ),
                    "",
                ));
                if selected_platform == PlatformKind::Slack {
                    card.items.push(Item {
                        value: t(lang, "panel.setup.slack.manifest"),
                        href: "/setup/slack-app-manifest.yml".into(),
                        ..Default::default()
                    });
                    card.items.push(Item {
                        value: t(lang, "panel.setup.slack.create_app"),
                        href: "https://api.slack.com/apps".into(),
                        ..Default::default()
                    });
                } else if selected_platform == PlatformKind::Telegram {
                    card.items.push(Item {
                        value: t(lang, "panel.setup.telegram.botfather"),
                        href: "https://t.me/BotFather".into(),
                        ..Default::default()
                    });
                } else if selected_platform == PlatformKind::Discord {
                    card.items.push(Item {
                        value: t(lang, "panel.setup.discord.portal"),
                        href: "https://discord.com/developers/applications".into(),
                        ..Default::default()
                    });
                }
                let fields = platform_credentials(selected_platform)
                    .iter()
                    .map(|key| {
                        let secret = !matches!(*key, "MATRIX_HOMESERVER" | "MATRIX_USER");
                        let mut input = field(
                            lang,
                            key,
                            &format!("panel.setup.field.{key}.label"),
                            if secret { "password" } else { "text" },
                            if secret { "" } else { get(key) },
                        );
                        input.help = t(lang, &format!("panel.setup.field.{key}.help"));
                        input.read_only = config_fields
                            .iter()
                            .any(|view| view.spec.key == *key && view.read_only);
                        if secret {
                            input.help.push('\n');
                            input.help.push_str(&t(lang, "panel.setup.keep_secret"));
                            input.autocomplete = "off".into();
                            input.configured = t(
                                lang,
                                if get(key).is_empty() {
                                    "panel.settings.empty"
                                } else {
                                    "panel.settings.configured"
                                },
                            );
                        }
                        input
                    })
                    .collect();
                let mut credentials = form(
                    "platform-form",
                    "/ui/setup/platform",
                    &info.csrf,
                    fields,
                    vec![button(lang, "panel.action.test_platform")],
                );
                credentials
                    .hidden
                    .push(("platform".into(), selected_platform.as_str().into()));
                card.forms.push(credentials);
            }
            3 => {
                card.description = t(lang, "panel.setup.provider");
                let mut provider = field(
                    lang,
                    "PROVIDER",
                    "panel.label.provider",
                    "select",
                    get("PROVIDER"),
                );
                provider.read_only = config_fields
                    .iter()
                    .any(|view| view.spec.key == "PROVIDER" && view.read_only);
                provider.choices = ["openrouter", "mistral"]
                    .into_iter()
                    .map(|value| views::Choice {
                        value: value.into(),
                        label: value.into(),
                        selected: get("PROVIDER") == value,
                    })
                    .collect();
                let mut model = field(lang, "MODEL", "panel.label.model", "text", get("MODEL"));
                model.read_only = config_fields
                    .iter()
                    .any(|view| view.spec.key == "MODEL" && view.read_only);
                let mut key = field(lang, "key", "panel.label.provider_key", "password", "");
                let key_name = if get("PROVIDER") == "mistral" {
                    "MISTRAL_KEY"
                } else {
                    "OPENROUTER_KEY"
                };
                // The generic key must accept a newly selected provider without JavaScript.
                key.read_only = provider.read_only
                    && config_fields
                        .iter()
                        .any(|view| view.spec.key == key_name && view.read_only);
                key.help = t(lang, "panel.setup.keep_secret");
                card.forms.push(form(
                    "provider-form",
                    "/ui/setup/provider",
                    &info.csrf,
                    vec![provider, key, model],
                    vec![button(lang, "panel.action.test_provider")],
                ));
            }
            4 => {
                card.description = t(lang, "panel.setup.fit");
                let mut fields = Vec::new();
                for &key in fit_fields(selected_platform) {
                    let mut input = field(
                        lang,
                        key,
                        &format!("panel.setup.field.{key}.label"),
                        "text",
                        get(key),
                    );
                    input.help = t(lang, &format!("panel.setup.field.{key}.help"));
                    input.read_only = config_fields
                        .iter()
                        .any(|view| view.spec.key == key && view.read_only);
                    if key == "OWNER_KEY" {
                        input.help.push('\n');
                        input.help.push_str(&t(
                            lang,
                            &format!("panel.setup.{}.owner", selected_platform.as_str()),
                        ));
                    }
                    if key == "BOT_LANG" {
                        input.kind = "select".into();
                        input.choices = ["en", "tr"]
                            .into_iter()
                            .map(|value| views::Choice {
                                value: value.into(),
                                label: t(
                                    lang,
                                    if value == "en" {
                                        "panel.language.en"
                                    } else {
                                        "panel.language.tr"
                                    },
                                ),
                                selected: get(key) == value,
                            })
                            .collect();
                    }
                    fields.push(input);
                }
                card.forms.push(form(
                    "fit-form",
                    "/ui/setup/fit",
                    &info.csrf,
                    fields,
                    vec![button(lang, "panel.action.continue")],
                ));
            }
            _ => {
                card.description = t(
                    lang,
                    &format!("panel.setup.{}.invite", selected_platform.as_str()),
                );
                if let Some(application) = application {
                    card.items.push(item(
                        lang,
                        "panel.label.application",
                        application.name,
                        &application.id,
                    ));
                    if let Some(workspace) = application.workspace {
                        card.items
                            .push(item(lang, "panel.setup.slack.workspace", workspace, ""));
                    }
                    if !application.invite_url.is_empty() {
                        card.items.push(Item {
                            label: t(lang, "panel.label.invite"),
                            value: t(
                                lang,
                                &format!("panel.setup.{}.add", selected_platform.as_str()),
                            ),
                            href: application.invite_url,
                            ..Default::default()
                        });
                    }
                    if selected_platform == PlatformKind::Telegram {
                        card.items.push(item(
                            lang,
                            "panel.setup.telegram.privacy_label",
                            t(
                                lang,
                                match application.privacy_mode {
                                    Some(true) => "panel.setup.telegram.privacy_on",
                                    Some(false) => "panel.setup.telegram.privacy_off",
                                    None => "panel.setup.telegram.privacy_unknown",
                                },
                            ),
                            "",
                        ));
                    }
                    card.items.push(item(
                        lang,
                        "panel.setup.verified_label",
                        t(lang, "panel.setup.not_ready"),
                        "",
                    ));
                } else {
                    page.error = t(lang, "panel.error.test");
                }
                let mut access = field(
                    lang,
                    "access_confirmed",
                    "panel.setup.access_confirmed",
                    "checkbox",
                    "on",
                );
                access.help = t(
                    lang,
                    &format!("panel.setup.{}.acknowledge", selected_platform.as_str()),
                );
                access.required = true;
                card.forms.push(form(
                    "complete-form",
                    "/ui/setup/complete",
                    &info.csrf,
                    vec![access],
                    vec![button(lang, "panel.action.start")],
                ));
            }
        }
    }
    if step > 2 {
        card.items.push(Item {
            value: t(lang, "panel.action.back"),
            href: format!("/setup?step={}&lang={}", step - 1, lang.code()),
            ..Default::default()
        });
    }
    if info.scope == auth::SessionScope::Admin {
        card.items.push(Item {
            value: t(lang, "panel.setup.more_platforms"),
            detail: t(lang, "panel.setup.more_platforms_help"),
            href: format!("/settings?lang={}", lang.code()),
            ..Default::default()
        });
    }
    page.sections.push(card);
    let response = html(&page)?;
    match issue {
        Some(issue) => cookie_response(response, &issue, app.secure),
        None => Ok(response),
    }
}

#[derive(Deserialize)]
struct AuthInput {
    csrf: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    current: String,
    #[serde(default)]
    token: String,
}
async fn claim(
    app: &App,
    headers: &HeaderMap,
    peer: SocketAddr,
    input: AuthInput,
) -> WebResult<auth::SessionIssue> {
    let session = token(headers)?;
    Ok(app
        .auth
        .claim(&input.token, &session, &input.csrf, peer.ip())?)
}
async fn ui_claim(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(input): Form<AuthInput>,
) -> WebResult<Response> {
    let issue = claim(&app, &headers, peer, input).await?;
    cookie_response(
        Redirect::to("/setup?step=1").into_response(),
        &issue,
        app.secure,
    )
}
async fn api_claim(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<AuthInput>,
) -> WebResult<Response> {
    let issue = claim(&app, &headers, peer, input).await?;
    cookie_response(Json(&issue.info).into_response(), &issue, app.secure)
}
async fn ui_protect(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(input): Form<AuthInput>,
) -> WebResult<Response> {
    let session = token(&headers)?;
    let issue = app
        .auth
        .create_admin(input.password, &session, &input.csrf, peer.ip())
        .await?;
    cookie_response(
        Redirect::to("/setup?step=2").into_response(),
        &issue,
        app.secure,
    )
}
async fn login(
    app: &App,
    headers: &HeaderMap,
    peer: SocketAddr,
    input: AuthInput,
) -> WebResult<auth::SessionIssue> {
    let session = token(headers)?;
    Ok(app
        .auth
        .login(input.password, &session, &input.csrf, peer.ip())
        .await?)
}
async fn ui_login(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(input): Form<AuthInput>,
) -> WebResult<Response> {
    let issue = login(&app, &headers, peer, input).await?;
    cookie_response(Redirect::to("/").into_response(), &issue, app.secure)
}
async fn api_login(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<AuthInput>,
) -> WebResult<Response> {
    let issue = login(&app, &headers, peer, input).await?;
    cookie_response(Json(&issue.info).into_response(), &issue, app.secure)
}
async fn logout(app: &App, headers: &HeaderMap, input: AuthInput) -> WebResult<Response> {
    let session = token(headers)?;
    app.auth.logout(&session, &input.csrf)?;
    app.drafts.lock().remove(&draft_key(&session));
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth::clear_cookie(app.secure))
            .map_err(|_| WebError::validation())?,
    );
    Ok(response)
}
async fn ui_logout(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Form(input): Form<AuthInput>,
) -> WebResult<Response> {
    logout(&app, &headers, input).await
}
async fn api_logout(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<AuthInput>,
) -> WebResult<Response> {
    logout(&app, &headers, input).await
}
async fn password(
    app: &App,
    headers: &HeaderMap,
    peer: SocketAddr,
    input: AuthInput,
) -> WebResult<auth::SessionIssue> {
    let session = token(headers)?;
    let issue = app
        .auth
        .change_password(
            input.current,
            input.password,
            &session,
            &input.csrf,
            peer.ip(),
        )
        .await?;
    app.drafts.lock().clear();
    Ok(issue)
}
async fn ui_password(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(input): Form<AuthInput>,
) -> WebResult<Response> {
    let issue = password(&app, &headers, peer, input).await?;
    cookie_response(Redirect::to("/account").into_response(), &issue, app.secure)
}
async fn api_password(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<AuthInput>,
) -> WebResult<Response> {
    let issue = password(&app, &headers, peer, input).await?;
    cookie_response(Json(&issue.info).into_response(), &issue, app.secure)
}
async fn auth_session(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> WebResult<Json<auth::SessionInfo>> {
    Ok(Json(app.auth.session(&token(&headers)?)?))
}

async fn setup_status(State(app): State<Arc<App>>, headers: HeaderMap) -> WebResult<Json<Value>> {
    let session = token(&headers)?;
    let info = app.auth.session(&session)?;
    if info.scope == auth::SessionScope::Preauth {
        return Err(auth::AuthError::Required.into());
    }
    let runtime = app.runtime.lock();
    Ok(Json(
        json!({"lifecycle":runtime.lifecycle,"scope":info.scope}),
    ))
}
async fn test_platform(
    app: &App,
    headers: &HeaderMap,
    peer: SocketAddr,
    input: &HashMap<String, String>,
) -> WebResult<setup::Application> {
    let session = mutation(app, headers, input.get("csrf").map_or("", String::as_str))?;
    app.auth.rate_limit(peer.ip())?;
    let _permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    let (platform, attempt, credentials) = {
        let config = app.config.lock();
        let mut drafts = app.drafts.lock();
        drafts.retain(|_, draft| draft.expires > Instant::now());
        let draft = drafts
            .entry(draft_key(&session))
            .or_insert_with(|| setup::Draft::new(&config));
        let platform = input
            .get("platform")
            .map(|value| setup::parse_platform(value))
            .transpose()
            .map_err(|_| WebError::validation())?
            .unwrap_or(draft.selected_platform);
        let attempt = reserve_platform(draft, platform);
        let config_fields = config.fields();
        let credentials: HashMap<String, String> = platform_credentials(platform)
            .iter()
            .map(|key| {
                let supplied = input.get(*key).filter(|value| !value.is_empty());
                if config_fields
                    .iter()
                    .any(|view| view.spec.key == *key && view.read_only)
                {
                    if supplied.is_some_and(|value| value != config.get(key)) {
                        return Err(WebError::validation());
                    }
                    return Ok(((*key).to_owned(), config.get(key).to_owned()));
                }
                let value = supplied
                    .or_else(|| draft.values.get(*key))
                    .cloned()
                    .unwrap_or_default();
                Ok(((*key).to_owned(), value))
            })
            .collect::<WebResult<_>>()?;
        (platform, attempt, credentials)
    };
    let application = setup::test_chat_platform(platform, &credentials)
        .await
        .map_err(|_| {
            WebError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "platform_test_failed",
                "panel.error.test",
            )
        })?;
    {
        let mut drafts = app.drafts.lock();
        let draft = drafts
            .get_mut(&draft_key(&session))
            .ok_or_else(WebError::validation)?;
        publish_platform(draft, attempt, application.clone(), credentials)?;
    }
    Ok(application)
}
async fn ui_platform(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(input): Form<HashMap<String, String>>,
) -> WebResult<Redirect> {
    test_platform(&app, &headers, peer, &input).await?;
    Ok(Redirect::to(&format!(
        "/setup?step=3&lang={}",
        lang(&input).code()
    )))
}
async fn api_platform(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<HashMap<String, String>>,
) -> WebResult<Json<setup::Application>> {
    Ok(Json(test_platform(&app, &headers, peer, &input).await?))
}
async fn test_provider(
    app: &App,
    headers: &HeaderMap,
    peer: SocketAddr,
    input: &HashMap<String, String>,
) -> WebResult<()> {
    let session = mutation(app, headers, input.get("csrf").map_or("", String::as_str))?;
    app.auth.rate_limit(peer.ip())?;
    let _permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    let (provider, key, model) = {
        let config = app.config.lock();
        let mut drafts = app.drafts.lock();
        let draft = drafts
            .entry(draft_key(&session))
            .or_insert_with(|| setup::Draft::new(&config));
        if draft.expires <= Instant::now() || draft.step() < 3 {
            return Err(WebError::validation());
        }
        draft.provider_verified = false;
        draft.fit_completed = false;
        let provider = input
            .get("PROVIDER")
            .cloned()
            .unwrap_or_else(|| config.selected_provider().to_string());
        let key_name = if provider == "mistral" {
            "MISTRAL_KEY"
        } else {
            "OPENROUTER_KEY"
        };
        let config_fields = config.fields();
        for (config_key, form_key) in [
            ("PROVIDER", "PROVIDER"),
            (key_name, "key"),
            ("MODEL", "MODEL"),
        ] {
            if config_fields
                .iter()
                .any(|view| view.spec.key == config_key && view.read_only)
                && input
                    .get(form_key)
                    .filter(|value| !value.is_empty())
                    .is_some_and(|value| value != config.get(config_key))
            {
                return Err(WebError::validation());
            }
        }
        let key = input
            .get("key")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| draft.values.get(key_name).cloned().unwrap_or_default());
        let model = input
            .get("MODEL")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                if provider == "mistral" {
                    crate::MISTRAL_MODEL.into()
                } else {
                    crate::OPENROUTER_MODEL.into()
                }
            });
        (provider, key, model)
    };
    setup::test_provider(&provider, &key, &model)
        .await
        .map_err(|_| {
            WebError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "provider_test_failed",
                "panel.error.test",
            )
        })?;
    {
        let mut drafts = app.drafts.lock();
        let draft = drafts
            .get_mut(&draft_key(&session))
            .ok_or_else(WebError::validation)?;
        draft.values.insert("PROVIDER".into(), provider.clone());
        draft.values.insert(
            if provider == "mistral" {
                "MISTRAL_KEY".into()
            } else {
                "OPENROUTER_KEY".into()
            },
            key,
        );
        draft.values.insert("MODEL".into(), model);
        draft.provider_verified = true;
        draft.fit_completed = false;
        draft.expires = Instant::now() + std::time::Duration::from_secs(900);
    }
    Ok(())
}
async fn ui_provider(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(input): Form<HashMap<String, String>>,
) -> WebResult<Redirect> {
    test_provider(&app, &headers, peer, &input).await?;
    Ok(Redirect::to(&format!(
        "/setup?step=4&lang={}",
        lang(&input).code()
    )))
}
async fn api_provider(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<HashMap<String, String>>,
) -> WebResult<Json<Value>> {
    test_provider(&app, &headers, peer, &input).await?;
    Ok(Json(json!({"verified":true,"completion_test":false})))
}
async fn ui_fit(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Form(input): Form<HashMap<String, String>>,
) -> WebResult<Redirect> {
    let session = mutation(&app, &headers, input.get("csrf").map_or("", String::as_str))?;
    let config_fields = app.config.lock().fields();
    let mut drafts = app.drafts.lock();
    let draft = drafts
        .get_mut(&draft_key(&session))
        .filter(|draft| draft.expires > Instant::now() && draft.step() >= 4)
        .ok_or_else(WebError::validation)?;
    draft.fit_completed = false;
    let mut values = HashMap::new();
    for &key in fit_fields(draft.selected_platform) {
        if let Some(value) = input.get(key) {
            if let Some(view) = config_fields
                .iter()
                .find(|view| view.spec.key == key && view.read_only)
            {
                if value != view.effective.value.as_deref().unwrap_or("") {
                    return Err(WebError::validation());
                }
                continue;
            }
            values.insert(
                key.to_owned(),
                crate::config::validate_value(key, value).map_err(|_| WebError::validation())?,
            );
        }
    }
    // Only the selected platform's access keys are accepted; other connections stay intact.
    draft.values.extend(values);
    draft.fit_completed = true;
    Ok(Redirect::to(&format!(
        "/setup?step=5&lang={}",
        lang(&input).code()
    )))
}
async fn complete(
    app: Arc<App>,
    headers: &HeaderMap,
    input: HashMap<String, String>,
) -> WebResult<()> {
    let session = mutation(&app, headers, input.get("csrf").map_or("", String::as_str))?;
    if input.get("access_confirmed").map(String::as_str) != Some("on") {
        return Err(WebError::validation());
    }
    let (patch, revision) = {
        let drafts = app.drafts.lock();
        let draft = drafts
            .get(&draft_key(&session))
            .filter(|draft| draft.expires > Instant::now() && draft.step() == 5)
            .ok_or_else(WebError::validation)?;
        let platform = draft.selected_platform;
        let provider_key = if draft
            .values
            .get("PROVIDER")
            .is_some_and(|value| value == "mistral")
        {
            "MISTRAL_KEY"
        } else {
            "OPENROUTER_KEY"
        };
        let patch: Patch = draft
            .values
            .iter()
            .filter(|(key, _)| {
                platform_credentials(platform).contains(&key.as_str())
                    || fit_fields(platform).contains(&key.as_str())
                    || matches!(key.as_str(), "PROVIDER" | "MODEL")
                    || key.as_str() == provider_key
            })
            .map(|(key, value)| {
                (
                    key.clone(),
                    PatchValue::Replace {
                        value: value.clone(),
                    },
                )
            })
            .collect();
        (patch, draft.revision.clone())
    };
    let permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    let previous;
    {
        let mut runtime = app.runtime.lock();
        if runtime.attempted
            || !matches!(
                runtime.lifecycle,
                Lifecycle::Unconfigured | Lifecycle::ConfigurationError
            )
        {
            return Err(WebError::new(
                StatusCode::CONFLICT,
                "already_started",
                "panel.error.starting",
            ));
        }
        previous = runtime.lifecycle;
        runtime
            .lifecycle
            .start()
            .map_err(|_| WebError::validation())?;
    }
    let app_save = app.clone();
    let next = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if *app_save.stop.borrow() {
            return Err(WebError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "stopping",
                "panel.state.stopping",
            ));
        }
        let mut config = app_save.config.lock();
        let fields = config.fields();
        let mut patch = patch;
        patch.retain(|key, _| {
            !fields
                .iter()
                .any(|field| field.spec.key == key && field.read_only)
        });
        let candidate = config
            .validate_patch(&patch)
            .map_err(|_| WebError::validation())?;
        super::register_secrets(&candidate).map_err(|_| WebError::validation())?;
        config.save_patch(&revision, &patch).map_err(|_| {
            WebError::new(
                StatusCode::CONFLICT,
                "config_save_failed",
                "panel.error.save",
            )
        })?;
        let next = config.reload().map_err(|_| WebError::validation())?;
        *config = next.clone();
        Ok(next)
    })
    .await
    .map_err(|_| WebError::validation())
    .and_then(|result| result);
    let next = match next {
        Ok(next) => next,
        Err(error) => {
            let mut runtime = app.runtime.lock();
            if runtime.lifecycle == Lifecycle::ConfiguredStarting {
                runtime.lifecycle = previous;
            }
            return Err(error);
        }
    };
    app.start.try_send(next).map_err(|_| {
        WebError::new(
            StatusCode::CONFLICT,
            "already_started",
            "panel.error.starting",
        )
    })?;
    app.drafts.lock().remove(&draft_key(&session));
    Ok(())
}
async fn ui_complete(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Form(input): Form<HashMap<String, String>>,
) -> WebResult<Redirect> {
    let language = lang(&input);
    complete(app, &headers, input).await?;
    Ok(Redirect::to(&format!("/?lang={}", language.code())))
}
async fn api_complete(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<HashMap<String, String>>,
) -> WebResult<Json<Value>> {
    complete(app, &headers, input).await?;
    Ok(Json(json!({"lifecycle":"configured_starting"})))
}

async fn account_page(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let session = admin(&app, &headers)?;
    let info = app.auth.session(&session)?;
    let lang = lang(&query);
    let mut page = page(lang, "panel.title.account", "account")?;
    page.authenticated = true;
    page.description = t(lang, "panel.account.recovery");
    let mut card = section(lang, "panel.title.account");
    card.forms.push(form(
        "password-form",
        "/ui/auth/password",
        &info.csrf,
        vec![
            field(
                lang,
                "current",
                "panel.label.current_password",
                "password",
                "",
            ),
            field(lang, "password", "panel.label.new_password", "password", ""),
        ],
        vec![button(lang, "panel.action.change_password")],
    ));
    card.forms.push(form(
        "logout-form",
        "/ui/auth/logout",
        &info.csrf,
        vec![],
        vec![button(lang, "panel.action.logout")],
    ));
    page.sections.push(card);
    html(&page)
}
async fn help_page(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    admin(&app, &headers)?;
    let lang = lang(&query);
    let mut page = page(lang, "panel.title.help", "help")?;
    page.authenticated = true;
    let mut card = section(lang, "panel.title.help");
    card.description = t(lang, "panel.help.body");
    page.sections.push(card);
    let mut commands = section(lang, "panel.help.commands");
    commands.description = t(lang, "panel.help.commands_body");
    page.sections.push(commands);
    html(&page)
}
async fn logs_page(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    admin(&app, &headers)?;
    let lang = lang(&query);
    let mut page = page(lang, "panel.title.logs", "logs")?;
    page.authenticated = true;
    page.description = t(lang, "panel.logs.body");
    page.log_stream_url = "/api/v1/logs/stream".into();
    let mut recent = section(lang, "panel.logs.recent");
    recent.items = crate::logging::snapshot(None, 100)
        .entries
        .into_iter()
        .rev()
        .map(|entry| Item {
            label: format!("{} {}", entry.timestamp, entry.level),
            value: entry.message,
            ..Default::default()
        })
        .collect();
    if recent.items.is_empty() {
        recent.description = t(lang, "panel.logs.no_recent");
    }
    page.sections.push(recent);
    html(&page)
}
#[derive(Deserialize)]
struct LogQuery {
    after: Option<u64>,
    limit: Option<usize>,
}
async fn logs_api(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<LogQuery>,
) -> WebResult<Json<crate::logging::Batch>> {
    admin(&app, &headers)?;
    Ok(Json(crate::logging::snapshot(
        query.after,
        query.limit.unwrap_or(100).min(512),
    )))
}
async fn log_stream(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<LogQuery>,
) -> WebResult<Response> {
    let session = token(&headers)?;
    app.auth.require_admin_passive(&session)?;
    let permit = app
        .streams
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    let after = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .or(query.after);
    let stream = futures_util::stream::unfold(
        (app, session, after, permit, VecDeque::new(), false),
        |(app, session, mut after, permit, mut pending, closed)| async move {
            if closed {
                return None;
            }
            if app.auth.require_admin_passive(&session).is_err() || *app.stop.borrow() {
                let name = if *app.stop.borrow() {
                    "stopped"
                } else {
                    "session_ended"
                };
                return Some((
                    Ok::<_, std::convert::Infallible>(
                        axum::response::sse::Event::default().event(name).data("{}"),
                    ),
                    (app, session, after, permit, VecDeque::new(), true),
                ));
            }
            if pending.is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if app.auth.require_admin_passive(&session).is_err() || *app.stop.borrow() {
                    let name = if *app.stop.borrow() {
                        "stopped"
                    } else {
                        "session_ended"
                    };
                    return Some((
                        Ok::<_, std::convert::Infallible>(
                            axum::response::sse::Event::default().event(name).data("{}"),
                        ),
                        (app, session, after, permit, VecDeque::new(), true),
                    ));
                }
                let batch = crate::logging::snapshot(after, 64);
                if batch.reset {
                    after = None;
                    pending.push_back(
                        axum::response::sse::Event::default()
                            .event("reset")
                            .data("{}"),
                    );
                }
                if batch.dropped > 0 {
                    pending.push_back(
                        axum::response::sse::Event::default()
                            .event("lag")
                            .data(json!({"dropped":batch.dropped}).to_string()),
                    );
                }
                for entry in batch.entries {
                    after = Some(entry.id);
                    pending.push_back(
                        axum::response::sse::Event::default()
                            .event("line")
                            .id(entry.id.to_string())
                            .data(serde_json::to_string(&entry).expect("serializable log entry")),
                    );
                }
            }
            let event = pending
                .pop_front()
                .unwrap_or_else(|| axum::response::sse::Event::default().comment("heartbeat"));
            Some((
                Ok::<_, std::convert::Infallible>(event),
                (app, session, after, permit, pending, false),
            ))
        },
    );
    Ok(Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}

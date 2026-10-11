use super::routes::{
    admin, button, field, form, html, lang, mutation, page, section, t, WebError, WebResult,
};
use super::views::{Button, Choice, Field, Item, Language};
use super::{auth, setup, App};
use crate::config::{Config, FieldKind, FieldView, Patch, PatchValue, Source};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Form, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

pub(super) async fn refresh(app: Arc<App>) -> WebResult<()> {
    let permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut config = app.config.lock();
        config.refresh_saved().map_err(WebError::config)?;
        super::register_secrets(&config).map_err(|_| WebError::validation())
    })
    .await
    .map_err(|_| WebError::validation())?
}

pub(super) async fn api(State(app): State<Arc<App>>, headers: HeaderMap) -> WebResult<Json<Value>> {
    admin(&app, &headers)?;
    refresh(app.clone()).await?;
    let config = app.config.lock();
    Ok(Json(
        json!({"fields":config.fields(),"revision":config.revision(),"unknown_keys":config.unknown_keys(),"config_path":config.path().display().to_string()}),
    ))
}

fn input(language: Language, view: &FieldView) -> Field {
    let value = if view.spec.secret {
        ""
    } else if view.read_only {
        view.effective.value.as_deref().unwrap_or("")
    } else {
        view.saved
            .value
            .as_deref()
            .filter(|value| !value.is_empty())
            .or(view.effective.value.as_deref())
            .unwrap_or("")
    };
    let mut input = field(
        language,
        view.spec.key,
        view.spec.label_key,
        if view.spec.secret { "password" } else { "text" },
        value,
    );
    let choices: &[&str] = match view.spec.kind {
        FieldKind::Bool => &["on", "off"],
        FieldKind::Provider => &["openrouter", "mistral"],
        FieldKind::Language => &["en", "tr"],
        FieldKind::SlackHistoryTier => &["internal", "restricted"],
        FieldKind::LogLevel => &["off", "error", "warn", "info", "debug", "trace"],
        FieldKind::LogColor => &["auto", "on", "off"],
        _ => &[],
    };
    if !choices.is_empty()
        || matches!(
            view.spec.kind,
            FieldKind::TelegramRooms | FieldKind::SlackChannels
        )
    {
        input.value =
            crate::config::validate_value(view.spec.key, &input.value).unwrap_or(input.value);
    }
    if !choices.is_empty() {
        input.kind = "select".into();
        input.choices = choices
            .iter()
            .map(|value| Choice {
                value: (*value).into(),
                label: match *value {
                    "on" => t(language, "panel.state.on"),
                    "off" => t(language, "panel.state.off"),
                    "en" => t(language, "panel.language.en"),
                    "tr" => t(language, "panel.language.tr"),
                    "auto" => t(language, "panel.settings.auto"),
                    _ => match view.spec.kind {
                        FieldKind::LogLevel => t(language, &format!("panel.logs.{value}")),
                        FieldKind::SlackHistoryTier => {
                            t(language, &format!("panel.settings.slack_history.{value}"))
                        }
                        _ => value.to_string(),
                    },
                },
                selected: input.value == *value,
            })
            .collect();
    }
    input.help = t(language, view.spec.help_key);
    input.read_only = view.read_only;
    input.source = t(
        language,
        match view.source {
            Source::Environment => "panel.field.environment",
            Source::File => "panel.field.file",
            Source::Default => "panel.field.default",
        },
    );
    if view.spec.secret {
        input.configured = t(
            language,
            if view.effective.configured {
                "panel.settings.configured"
            } else {
                "panel.settings.empty"
            },
        );
    }
    input.effective = if view.spec.secret {
        input.configured.clone()
    } else {
        view.effective.value.clone().unwrap_or_default()
    };
    input.saved = if view.spec.secret {
        t(
            language,
            if view.saved.configured {
                "panel.settings.configured"
            } else {
                "panel.settings.empty"
            },
        )
    } else {
        view.saved.value.clone().unwrap_or_default()
    };
    input.default = view.default.value.clone().unwrap_or_default();
    input
}

pub(super) async fn view(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let session = admin(&app, &headers)?;
    let info = app.auth.session(&session)?;
    refresh(app.clone()).await?;
    let language = lang(&query);
    let mut page = page(language, "panel.title.settings", "settings")?;
    page.authenticated = true;
    page.description = t(language, "panel.settings.body");
    if query.get("validated").is_some_and(|value| value == "1") {
        page.notice = t(language, "panel.settings.validated");
    }
    if query.get("saved").is_some_and(|value| value == "1") {
        page.notice = t(language, "panel.settings.restart");
    }
    let config = app.config.lock();
    let views = config.fields();
    if views.iter().any(|view| view.pending_restart) {
        page.notice = t(language, "panel.settings.pending");
    }
    for (group, keys, advanced) in [
        (
            "platform",
            &[
                "DISCORD_TOKEN",
                "TELEGRAM_TOKEN",
                "TELEGRAM_ROOMS",
                "SLACK_APP_TOKEN",
                "SLACK_BOT_TOKEN",
                "SLACK_CHANNELS",
                "SLACK_HISTORY_TIER",
                "SLACK_TEST_CHANNEL",
            ][..],
            false,
        ),
        (
            "provider",
            &["PROVIDER", "OPENROUTER_KEY", "MISTRAL_KEY", "MODEL"][..],
            false,
        ),
        (
            "behavior",
            &["BOT_LANG", "TZ_OFFSET", "IMAGE_ANALYSIS", "STEALTH"][..],
            false,
        ),
        (
            "news",
            &["NEWS_RSS", "NEWS_CHANNEL", "FIRECRAWL_KEY"][..],
            true,
        ),
        (
            "privacy",
            &["OWNER_KEY", "OWNER_ID", "GUILD_ID", "CHANNELS", "API_URL"][..],
            true,
        ),
        (
            "diagnostics",
            &["DEBUG_CHANNEL", "LOG_LEVEL", "LOG_COLOR"][..],
            true,
        ),
        (
            "panel",
            &[
                "PANEL_ENABLED",
                "PANEL_BIND",
                "PANEL_PORT",
                "PANEL_CONFIG_PATH",
                "PANEL_PUBLIC_URL",
                "PANEL_TRUSTED_PROXIES",
            ][..],
            true,
        ),
    ] {
        let mut card = section(language, &format!("panel.settings.{group}"));
        card.id = format!("settings-{group}");
        card.advanced = advanced;
        if group == "provider" {
            card.description = t(language, "panel.settings.model_override");
        }
        let mut fields = Vec::new();
        for key in keys {
            if let Some(view) = views.iter().find(|view| view.spec.key == *key) {
                fields.push(input(language, view));
                if view.spec.secret && !view.read_only {
                    let mut clear = field(
                        language,
                        &format!("clear_{key}"),
                        "panel.settings.clear_secret",
                        "checkbox",
                        "on",
                    );
                    clear.label = format!(
                        "{} · {}",
                        t(language, "panel.settings.clear_secret"),
                        t(language, view.spec.label_key)
                    );
                    fields.push(clear);
                }
            }
        }
        let mut edit = form(
            &format!("settings-{group}-form"),
            "/ui/settings",
            &info.csrf,
            fields,
            vec![
                button(language, "panel.action.save"),
                Button {
                    label: t(language, "panel.action.validate"),
                    name: "operation".into(),
                    value: "validate".into(),
                    secondary: true,
                    ..Default::default()
                },
            ],
        );
        edit.hidden
            .push(("revision".into(), config.revision().into()));
        card.forms.push(edit);
        page.sections.push(card);
    }
    let mut tests = section(language, "panel.settings.tests");
    tests.description = t(language, "panel.settings.tests_help");
    tests.forms.push(form(
        "settings-platform-test",
        "/ui/settings/test-platform",
        &info.csrf,
        vec![field(
            language,
            "DISCORD_TOKEN",
            "panel.settings.DISCORD_TOKEN.label",
            "password",
            "",
        )],
        vec![button(language, "panel.action.test_platform")],
    ));
    let mut provider = field(language, "PROVIDER", "panel.label.provider", "select", "");
    provider.choices = ["", "openrouter", "mistral"]
        .into_iter()
        .map(|value| Choice {
            value: value.into(),
            label: if value.is_empty() {
                t(language, "panel.settings.use_saved")
            } else {
                value.into()
            },
            selected: value.is_empty(),
        })
        .collect();
    tests.forms.push(form(
        "settings-provider-test",
        "/ui/settings/test-provider",
        &info.csrf,
        vec![
            provider,
            field(language, "key", "panel.label.provider_key", "password", ""),
            field(language, "MODEL", "panel.label.model", "text", ""),
        ],
        vec![button(language, "panel.action.test_provider")],
    ));
    page.sections.push(tests);
    let unknown = config.unknown_keys();
    if !unknown.is_empty() {
        let mut card = section(language, "panel.settings.unknown");
        card.description = t(language, "panel.settings.unknown_help");
        card.items = unknown
            .into_iter()
            .map(|key| Item {
                value: key.into(),
                ..Default::default()
            })
            .collect();
        page.sections.push(card);
    }
    html(&page)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SettingsInput {
    csrf: String,
    revision: String,
    patch: Patch,
}
async fn save(
    app: Arc<App>,
    headers: &HeaderMap,
    input: SettingsInput,
    validate_only: bool,
) -> WebResult<Value> {
    mutation(&app, headers, &input.csrf)?;
    let permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    tokio::task::spawn_blocking(move||{
        let _permit=permit;
        if *app.stop.borrow(){return Err(WebError::new(StatusCode::SERVICE_UNAVAILABLE,"stopping","panel.state.stopping"));}
        let mut config=app.config.lock();let candidate=config.validate_patch(&input.patch).map_err(WebError::config)?;
        if !validate_only{super::register_secrets(&candidate).map_err(|_|WebError::validation())?;config.save_patch(&input.revision,&input.patch).map_err(WebError::config)?;}
        Ok(json!({"valid":true,"restart_required":!validate_only&&config.fields().iter().any(|view|view.pending_restart),"revision":config.revision()}))
    }).await.map_err(|_|WebError::validation())?
}
pub(super) async fn patch(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<SettingsInput>,
) -> WebResult<Json<Value>> {
    Ok(Json(save(app, &headers, input, false).await?))
}
pub(super) async fn validate(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<SettingsInput>,
) -> WebResult<Json<Value>> {
    Ok(Json(save(app, &headers, input, true).await?))
}
pub(super) async fn ui(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Form(mut input): Form<HashMap<String, String>>,
) -> WebResult<Response> {
    let csrf = input.remove("csrf").unwrap_or_default();
    let revision = input.remove("revision").unwrap_or_default();
    let validate_only = input.remove("operation").as_deref() == Some("validate");
    let config_fields = app.config.lock().fields();
    let mut patch = Patch::new();
    for view in config_fields {
        if view.read_only {
            continue;
        }
        let clear = input
            .get(&format!("clear_{}", view.spec.key))
            .is_some_and(|value| value == "on");
        let value = input.remove(view.spec.key);
        if value.is_none() && !clear {
            continue;
        }
        let value = value.unwrap_or_default();
        let operation = if clear {
            PatchValue::Clear
        } else if view.spec.secret && value.is_empty() {
            PatchValue::Keep
        } else {
            PatchValue::Replace { value }
        };
        patch.insert(view.spec.key.to_owned(), operation);
    }
    save(
        app,
        &headers,
        SettingsInput {
            csrf,
            revision,
            patch,
        },
        validate_only,
    )
    .await?;
    Ok(Redirect::to(if validate_only {
        "/settings?validated=1"
    } else {
        "/settings?saved=1"
    })
    .into_response())
}

async fn saved_config(app: Arc<App>) -> WebResult<Config> {
    tokio::task::spawn_blocking(move || app.config.lock().reload().map_err(WebError::config))
        .await
        .map_err(|_| WebError::validation())?
}

async fn check_platform(
    app: Arc<App>,
    headers: &HeaderMap,
    peer: SocketAddr,
    input: &HashMap<String, String>,
) -> WebResult<setup::Application> {
    mutation(&app, headers, input.get("csrf").map_or("", String::as_str))?;
    app.auth.rate_limit(peer.ip())?;
    let _permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    let config = saved_config(app).await?;
    let token = input
        .get("DISCORD_TOKEN")
        .filter(|value| !value.is_empty())
        .map_or_else(|| config.get("DISCORD_TOKEN"), String::as_str);
    setup::test_platform(token).await.map_err(|_| {
        WebError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "platform_test_failed",
            "panel.error.test",
        )
    })
}

async fn check_provider(
    app: Arc<App>,
    headers: &HeaderMap,
    peer: SocketAddr,
    input: &HashMap<String, String>,
) -> WebResult<()> {
    mutation(&app, headers, input.get("csrf").map_or("", String::as_str))?;
    app.auth.rate_limit(peer.ip())?;
    let _permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| auth::AuthError::Busy)?;
    let config = saved_config(app).await?;
    let provider = input
        .get("PROVIDER")
        .filter(|value| !value.is_empty())
        .map_or_else(|| config.selected_provider(), String::as_str);
    let key_name = if provider == "mistral" {
        "MISTRAL_KEY"
    } else {
        "OPENROUTER_KEY"
    };
    let key = input
        .get("key")
        .or_else(|| input.get(key_name))
        .filter(|value| !value.is_empty())
        .map_or_else(|| config.get(key_name), String::as_str);
    let model = input
        .get("MODEL")
        .filter(|value| !value.is_empty())
        .map_or_else(|| config.get("MODEL"), String::as_str);
    setup::test_provider(provider, key, model)
        .await
        .map_err(|_| {
            WebError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "provider_test_failed",
                "panel.error.test",
            )
        })
}

fn tested(language: Language, application: Option<setup::Application>) -> WebResult<Response> {
    let mut page = page(language, "panel.title.settings", "settings")?;
    page.authenticated = true;
    page.notice = t(language, "panel.settings.test_passed");
    let mut result = section(language, "panel.settings.tests");
    if let Some(application) = application {
        result.items.push(Item {
            label: t(language, "panel.label.application"),
            value: format!("{} · {}", application.name, application.id),
            ..Default::default()
        });
    }
    result.items.push(Item {
        value: t(language, "panel.action.back"),
        href: format!("/settings?lang={}", language.code()),
        ..Default::default()
    });
    page.sections.push(result);
    html(&page)
}

pub(super) async fn platform_ui(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Form(input): Form<HashMap<String, String>>,
) -> WebResult<Response> {
    let application = check_platform(app, &headers, peer, &input).await?;
    tested(lang(&query), Some(application))
}
pub(super) async fn provider_ui(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Form(input): Form<HashMap<String, String>>,
) -> WebResult<Response> {
    check_provider(app, &headers, peer, &input).await?;
    tested(lang(&query), None)
}
pub(super) async fn platform_api(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<HashMap<String, String>>,
) -> WebResult<Json<setup::Application>> {
    Ok(Json(check_platform(app, &headers, peer, &input).await?))
}
pub(super) async fn provider_api(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<HashMap<String, String>>,
) -> WebResult<Json<Value>> {
    check_provider(app, &headers, peer, &input).await?;
    Ok(Json(json!({"verified":true,"completion_test":false})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_inputs_are_localized_write_only_and_typed_without_changing_discord() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(".env");
        std::fs::write(
            &path,
            "DISCORD_TOKEN=discord-private-token\nTELEGRAM_TOKEN=telegram-private-token\nTELEGRAM_ROOMS=-100123\nSLACK_APP_TOKEN=app-private-token\nSLACK_BOT_TOKEN=bot-private-token\nSLACK_CHANNELS=C123,G456,C123\nSLACK_TEST_CHANNEL=G456\nOPENROUTER_KEY=key\n",
        )
        .unwrap();
        let config = Config::load(path, &HashMap::new()).unwrap();
        let fields = config.fields();
        for language in [Language::En, Language::Tr] {
            for key in [
                "DISCORD_TOKEN",
                "TELEGRAM_TOKEN",
                "SLACK_APP_TOKEN",
                "SLACK_BOT_TOKEN",
            ] {
                let view = fields.iter().find(|view| view.spec.key == key).unwrap();
                let input = input(language, view);
                assert_eq!(input.kind, "password");
                assert!(input.value.is_empty());
                assert_eq!(input.configured, t(language, "panel.settings.configured"));
                assert_eq!(input.effective, input.configured);
                assert_eq!(input.saved, input.configured);
                assert!(input.default.is_empty());
                assert!(!input.label.starts_with("panel."));
                assert!(!input.help.starts_with("panel."));
                assert!(!input.help.contains("private-token"));
            }
            let view = fields
                .iter()
                .find(|view| view.spec.key == "TELEGRAM_ROOMS")
                .unwrap();
            let input = input(language, view);
            assert_eq!(input.kind, "text");
            assert_eq!(input.value, "telegram:room:-100123");
            assert!(input.configured.is_empty());
            assert!(!input.label.starts_with("panel."));
            assert!(!input.help.starts_with("panel."));
            for (key, expected) in [
                ("SLACK_CHANNELS", "C123,G456"),
                ("SLACK_TEST_CHANNEL", "G456"),
            ] {
                let view = fields.iter().find(|view| view.spec.key == key).unwrap();
                let input = super::input(language, view);
                assert_eq!(input.kind, "text");
                assert_eq!(input.value, expected);
                assert!(input.configured.is_empty());
                assert!(!input.label.starts_with("panel."));
                assert!(!input.help.starts_with("panel."));
            }
            let view = fields
                .iter()
                .find(|view| view.spec.key == "SLACK_HISTORY_TIER")
                .unwrap();
            let input = super::input(language, view);
            assert_eq!(input.kind, "select");
            assert_eq!(input.value, "internal");
            assert_eq!(input.choices.len(), 2);
            assert_eq!(input.choices[0].value, "internal");
            assert!(input.choices[0].selected);
            assert_eq!(input.choices[1].value, "restricted");
            assert!(!input.choices[1].selected);
            assert!(!input.label.starts_with("panel."));
            assert!(!input.help.starts_with("panel."));
            for choice in &input.choices {
                assert_eq!(
                    choice.label,
                    t(
                        language,
                        &format!("panel.settings.slack_history.{}", choice.value)
                    )
                );
            }
        }
    }
}

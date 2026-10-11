use super::routes::{
    admin, button, field, form, html, item, lang, mutation, page, section, t, WebError, WebResult,
};
use super::views::{Form as ViewForm, Item, Language, Page};
use super::App;
use crate::{memory, BotError};
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

const TEMPERAMENT: &str = "huy.md";
const SECTIONS: [(&str, &str); 5] = [
    ("temperament", TEMPERAMENT),
    ("profile", "profil.md"),
    ("self", "kendim.md"),
    ("corrections", "duzeltmeler.md"),
    ("growth", "gelisim.md"),
];

pub(super) fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/personality", get(view))
        .route("/api/v1/personality", get(api))
        .route("/api/v1/personality/temperament", axum::routing::put(put))
        .route(
            "/ui/personality/temperament",
            post(save_form).layer(DefaultBodyLimit::max(96 * 1024)),
        )
}

fn storage_error() -> WebError {
    WebError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "memory_unavailable",
        "panel.personality.storage_error",
    )
}

fn record_value(record: Option<memory::RecordSnapshot>, editable: bool) -> Value {
    match record {
        Some(record) => {
            json!({"key":record.key,"content":record.content,"revision":record.revision,"modified":record.modified,"exists":true,"editable":editable})
        }
        None => {
            json!({"content":null,"revision":0,"modified":null,"exists":false,"editable":editable})
        }
    }
}

async fn snapshot(app: &App) -> WebResult<Value> {
    let bot = app.runtime.lock().bot.clone();
    // Do not freeze Lang::current before first-run setup has activated BOT_LANG.
    let (bot_language, active) = if bot.is_some() {
        (crate::lang::Lang::current(), true)
    } else {
        let language = if app.config.lock().get("BOT_LANG") == "tr" {
            crate::lang::Lang::Tr
        } else {
            crate::lang::Lang::En
        };
        (language, false)
    };
    let permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| storage_error())?;
    tokio::task::spawn_blocking(move || -> Result<Value, BotError> {
        let _permit = permit;
        let code = match bot_language { crate::lang::Lang::En => "en", crate::lang::Lang::Tr => "tr" };
        let mut result = json!({"core":{"content":crate::prompts::get(bot_language).personality,"editable":false,"compiled":true,"language":code,"active":active},"mood":{"editable":false,"scope":"conversation","href":"/"}});
        for (name, key) in SECTIONS {
            result[name] = record_value(memory::read_versioned(key)?, key == TEMPERAMENT);
            result[name]["key"] = json!(key);
        }
        if let Some(bot) = bot {
            let state = bot.state();
            result["growth"]["live"] = json!({"birth":state.growth.birth,"days":crate::growth::days(&state.growth).max(0),"finished_chats":state.growth.chats,"messages":state.growth.messages,"stage":state.growth.stage,"name":state.growth.name});
        }
        Ok(result)
    }).await.map_err(|_| storage_error())?.map_err(|_| storage_error())
}

async fn api(State(app): State<Arc<App>>, headers: HeaderMap) -> WebResult<Response> {
    admin(&app, &headers)?;
    Ok(Json(snapshot(&app).await?).into_response())
}

fn editor(language: Language, csrf: &str, content: &str, revision: u64) -> ViewForm {
    let mut content_field = field(
        language,
        "content",
        "panel.personality.temperament",
        "textarea",
        content,
    );
    content_field.help = t(language, "panel.personality.manual_notice");
    let mut editor = form(
        "temperament-editor",
        "/ui/personality/temperament",
        csrf,
        vec![content_field],
        vec![button(language, "panel.personality.save")],
    );
    editor
        .hidden
        .push(("revision".into(), revision.to_string()));
    editor
}

fn render_snapshot(
    language: Language,
    csrf: &str,
    snapshot: &Value,
    saved: bool,
) -> WebResult<Page> {
    let mut result = page(language, "panel.title.personality", "personality")?;
    result.authenticated = true;
    result.description = t(language, "panel.personality.description");
    if saved {
        result.notice = t(language, "panel.personality.saved");
    }
    for name in [
        "core",
        "temperament",
        "profile",
        "self",
        "corrections",
        "growth",
    ] {
        let key = format!("panel.personality.{name}");
        let mut block = section(language, &key);
        block.id = name.into();
        block.description = t(language, &format!("{key}.source"));
        if name == "core" {
            block.description.push_str(&format!(
                "\nBOT_LANG: {}",
                snapshot["core"]["language"].as_str().unwrap_or("")
            ));
            if snapshot["core"]["active"] == false {
                block.description.push_str(&format!(
                    "\n{}",
                    t(language, "panel.personality.core_preview")
                ));
            }
        }
        if name == "temperament" {
            block.forms.push(editor(
                language,
                csrf,
                snapshot[name]["content"].as_str().unwrap_or(""),
                snapshot[name]["revision"].as_u64().unwrap_or(0),
            ));
            block.items.push(item(
                language,
                "panel.personality.revision",
                snapshot[name]["revision"].as_u64().unwrap_or(0),
                "",
            ));
        } else {
            let mut text = field(
                language,
                name,
                &key,
                "textarea",
                snapshot[name]["content"].as_str().unwrap_or(""),
            );
            text.read_only = true;
            if snapshot[name]["content"].is_null() {
                text.help = t(language, "panel.personality.no_record");
            }
            block.forms.push(ViewForm {
                id: format!("personality-{name}"),
                fields: vec![text],
                ..Default::default()
            });
        }
        if name == "growth" && snapshot[name]["live"].is_object() {
            for (field, label) in [
                ("days", "panel.label.days"),
                ("finished_chats", "panel.label.finished_chats"),
                ("messages", "panel.label.messages"),
            ] {
                block.items.push(item(
                    language,
                    label,
                    snapshot[name]["live"][field].as_u64().unwrap_or(0),
                    "",
                ));
            }
            block.items.push(item(
                language,
                "panel.personality.name",
                snapshot[name]["live"]["name"]
                    .as_str()
                    .map_or_else(|| t(language, "panel.state.no_name"), str::to_owned),
                "",
            ));
        }
        result.sections.push(block);
    }
    let mut mood = section(language, "panel.personality.mood");
    mood.description = t(language, "panel.personality.mood.source");
    mood.items.push(Item {
        label: t(language, "panel.personality.mood"),
        value: t(language, "panel.personality.view_conversations"),
        href: format!("/?lang={}", language.code()),
        ..Default::default()
    });
    result.sections.push(mood);
    let mut refresh = section(language, "panel.personality.refresh");
    refresh.description = t(language, "panel.personality.refresh_warning");
    refresh.items.push(Item {
        label: t(language, "panel.personality.refresh"),
        value: t(language, "panel.personality.refresh"),
        href: format!("/actions?lang={}#refresh_agents", language.code()),
        ..Default::default()
    });
    result.sections.push(refresh);
    Ok(result)
}

async fn view(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    let session = admin(&app, &headers)?;
    let csrf = app.auth.session(&session)?.csrf;
    let snapshot = snapshot(&app).await?;
    html(&render_snapshot(
        lang(&query),
        &csrf,
        &snapshot,
        query.get("saved").is_some_and(|value| value == "1"),
    )?)
}

fn expected_revision(headers: &HeaderMap) -> WebResult<u64> {
    let value = headers.get(header::IF_MATCH).ok_or_else(|| {
        WebError::new(
            StatusCode::PRECONDITION_REQUIRED,
            "revision_required",
            "panel.personality.revision_required",
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    content: String,
}

fn validate_content(content: &str) -> WebResult<()> {
    if content.len() > 24 * 1024 || content.contains('\0') {
        return Err(WebError::validation());
    }
    Ok(())
}

async fn replace(
    app: &Arc<App>,
    expected: u64,
    content: String,
) -> WebResult<Result<memory::RecordSnapshot, memory::MutationError>> {
    validate_content(&content)?;
    let permit = app
        .tests
        .clone()
        .try_acquire_owned()
        .map_err(|_| storage_error())?;
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let runtime = super::routes::edit_runtime(&app)?;
        Ok(memory::replace_versioned(
            runtime.bot.as_deref(),
            TEMPERAMENT,
            expected,
            &content,
        ))
    })
    .await
    .map_err(|_| storage_error())?
}

fn etag(response: &mut Response, revision: u64) {
    if let Ok(value) = HeaderValue::from_str(&format!("\"{revision}\"")) {
        response.headers_mut().insert(header::ETAG, value);
    }
}

fn conflict_json(language: Language, current: memory::RecordSnapshot) -> Response {
    let revision = current.revision;
    let mut response = StatusCode::PRECONDITION_FAILED.into_response();
    response.extensions_mut().insert(super::routes::ErrorDetails(json!({"code":"revision_conflict","message":t(language,"panel.personality.conflict"),"field_errors":{},"current":record_value(Some(current),true)})));
    etag(&mut response, revision);
    response
}

async fn put(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Json(edit): Json<Edit>,
) -> WebResult<Response> {
    let csrf = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    mutation(&app, &headers, csrf)?;
    let expected = expected_revision(&headers)?;
    match replace(&app, expected, edit.content).await? {
        Ok(record) => {
            let revision = record.revision;
            let mut response = Json(record_value(Some(record), true)).into_response();
            etag(&mut response, revision);
            Ok(response)
        }
        Err(memory::MutationError::Conflict(current)) => Ok(conflict_json(lang(&query), current)),
        Err(memory::MutationError::Missing) => Err(WebError::new(
            StatusCode::NOT_FOUND,
            "record_missing",
            "panel.personality.no_record",
        )),
        Err(memory::MutationError::Storage(_)) => Err(storage_error()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeEdit {
    csrf: String,
    lang: String,
    revision: u64,
    content: String,
}

async fn save_form(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Form(edit): Form<NativeEdit>,
) -> WebResult<Response> {
    mutation(&app, &headers, &edit.csrf)?;
    let language = Language::parse(&edit.lang);
    let draft = edit.content.clone();
    match replace(&app, edit.revision, edit.content).await? {
        Ok(_) => Ok(axum::response::Redirect::to(&format!(
            "/personality?lang={}&saved=1",
            language.code()
        ))
        .into_response()),
        Err(memory::MutationError::Conflict(current)) => {
            let mut result = page(language, "panel.title.personality", "personality")?;
            result.authenticated = true;
            result.error = t(language, "panel.personality.conflict");
            let mut block = section(language, "panel.personality.conflict_title");
            block.description = t(language, "panel.personality.conflict_help");
            let mut your_draft = field(
                language,
                "draft",
                "panel.personality.draft",
                "textarea",
                &draft,
            );
            your_draft.read_only = true;
            let mut live = field(
                language,
                "current",
                "panel.personality.current",
                "textarea",
                &current.content,
            );
            live.read_only = true;
            block.forms.push(ViewForm {
                id: "temperament-conflict".into(),
                fields: vec![your_draft, live],
                ..Default::default()
            });
            block.items.push(Item {
                label: t(language, "panel.personality.reload"),
                value: t(language, "panel.personality.reload"),
                href: format!("/personality?lang={}", language.code()),
                ..Default::default()
            });
            result.sections.push(block);
            let mut response = html(&result)?;
            *response.status_mut() = StatusCode::PRECONDITION_FAILED;
            response
                .extensions_mut()
                .insert(super::routes::PreserveErrorBody);
            etag(&mut response, current.revision);
            Ok(response)
        }
        Err(memory::MutationError::Missing) => Err(WebError::new(
            StatusCode::NOT_FOUND,
            "record_missing",
            "panel.personality.no_record",
        )),
        Err(memory::MutationError::Storage(_)) => Err(storage_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperament_is_the_only_writable_key() {
        assert_eq!(
            SECTIONS
                .iter()
                .filter(|(_, key)| *key == TEMPERAMENT)
                .count(),
            1
        );
        for key in ["profil.md", "kendim.md", "duzeltmeler.md", "gelisim.md"] {
            assert_ne!(key, TEMPERAMENT);
        }
        assert!(
            serde_json::from_value::<Edit>(json!({"key":"gelisim.md","content":"changed"}))
                .is_err()
        );
    }

    #[test]
    fn revisions_require_one_strong_etag() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            expected_revision(&headers)
                .err()
                .unwrap()
                .into_response()
                .status(),
            StatusCode::PRECONDITION_REQUIRED
        );
        for value in ["*", "W/\"1\"", "1", "\"1\", \"2\"", "\"-1\""] {
            headers.insert(header::IF_MATCH, HeaderValue::from_str(value).unwrap());
            assert!(expected_revision(&headers).is_err(), "{value}");
        }
        headers.insert(header::IF_MATCH, HeaderValue::from_static("\"17\""));
        assert_eq!(
            expected_revision(&headers).unwrap_or_else(|_| panic!("valid revision rejected")),
            17
        );
    }

    #[tokio::test]
    async fn temperament_cas_preserves_current_text_on_conflict() {
        memory::test::ensure_test_db();
        // All real writes use the canonical temperament key and shared CAS service.
        let (app, _directory) = super::super::tests::fixture().await;
        memory::write_result(TEMPERAMENT, "temperament fixture").unwrap();
        let original = memory::read_versioned(TEMPERAMENT).unwrap().unwrap();
        let saved = replace(&app, original.revision, "Yeni huy — café".into())
            .await
            .unwrap_or_else(|_| panic!("CAS service unavailable"))
            .unwrap_or_else(|_| panic!("initial CAS failed"));
        assert!(saved.revision > original.revision);
        let conflict = replace(&app, original.revision, "stale draft".into())
            .await
            .unwrap_or_else(|_| panic!("CAS service unavailable"));
        match conflict {
            Err(memory::MutationError::Conflict(current)) => {
                assert_eq!(current.content, saved.content);
                assert_eq!(current.revision, saved.revision);
            }
            _ => panic!("stale edit did not conflict"),
        }
        assert_eq!(
            memory::read_versioned(TEMPERAMENT)
                .unwrap()
                .unwrap()
                .content,
            saved.content
        );
        use axum::body::to_bytes;
        use tower::ServiceExt;
        let owner = super::super::tests::owner(&app).await;
        let router = super::super::routes::router(app.clone());
        let mut request = super::super::tests::request(
            "PUT",
            "/api/v1/personality/temperament?lang=en",
            Some(&owner.token),
            "{\"content\":\"new draft\"}",
        );
        request.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        request.headers_mut().insert(
            "x-csrf-token",
            HeaderValue::from_str(&owner.info.csrf).unwrap(),
        );
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
        let mut request = super::super::tests::request(
            "PUT",
            "/api/v1/personality/temperament?lang=tr",
            Some(&owner.token),
            "{\"content\":\"stale API draft\"}",
        );
        request.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        request.headers_mut().insert(
            "x-csrf-token",
            HeaderValue::from_str(&owner.info.csrf).unwrap(),
        );
        request.headers_mut().insert(
            header::IF_MATCH,
            HeaderValue::from_str(&format!("\"{}\"", original.revision)).unwrap(),
        );
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let conflict: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(conflict["code"], "revision_conflict");
        assert_eq!(conflict["current"]["content"], saved.content);
        assert!(conflict["request_id"].is_string());
        assert_eq!(
            conflict["message"],
            t(Language::Tr, "panel.personality.conflict")
        );
        let body = format!(
            "csrf={}&lang=en&revision={}&content=draft%20from%20form",
            owner.info.csrf, original.revision
        );
        let response = router
            .clone()
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/personality/temperament?lang=en",
                Some(&owner.token),
                &body,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("draft from form"));
        assert!(body.contains("Yeni huy — café"));
        let response = router
            .oneshot(super::super::tests::request(
                "GET",
                "/api/v1/personality",
                Some(&owner.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let snapshot: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(snapshot["temperament"]["editable"], true);
        for name in ["core", "profile", "self", "corrections", "growth"] {
            assert_eq!(snapshot[name]["editable"], false, "{name}");
        }
        assert_eq!(snapshot["self"]["key"], "kendim.md");
        let temperament = "ş".repeat(6000);
        let body = format!(
            "csrf={}&lang=tr&revision={}&content={}",
            owner.info.csrf,
            saved.revision,
            "%C5%9F".repeat(6000)
        );
        assert!(body.len() > 32 * 1024);
        assert!(temperament.len() < 24 * 1024);
        let response = super::super::routes::router(app)
            .oneshot(super::super::tests::request(
                "POST",
                "/ui/personality/temperament?lang=tr",
                Some(&owner.token),
                &body,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            memory::read_versioned(TEMPERAMENT)
                .unwrap()
                .unwrap()
                .content,
            temperament
        );
    }
}

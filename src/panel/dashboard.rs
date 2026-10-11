use super::routes::{admin, html, item, lang, page, section, t, token, WebResult};
use super::views::{Item, Language};
use super::App;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) async fn snapshot(app: &App, language: Language) -> Value {
    let (lifecycle, bot, manager, cache) = {
        let runtime = app.runtime.lock();
        (
            runtime.lifecycle,
            runtime.bot.clone(),
            runtime.manager.clone(),
            runtime.cache.clone(),
        )
    };
    let connection = if let Some(manager) = manager {
        let runners = manager.runners.lock().await;
        if runners
            .values()
            .any(|runner| runner.stage == serenity::gateway::ConnectionStage::Connected)
        {
            "connected"
        } else if runners.is_empty() {
            "offline"
        } else {
            "reconnecting"
        }
    } else {
        "offline"
    };
    let mut result = {
        let config = app.config.lock();
        json!({"lifecycle":lifecycle,"panel":"healthy","connection":connection,"spaces":cache.map_or(0,|cache|cache.guild_count()),"provider":config.selected_provider(),"model":config.get("MODEL"),"model_health":"not_reported","pending_restart":config.fields().iter().any(|field|field.pending_restart),"updated_at":crate::now_unix()})
    };
    if let Some(bot) = bot {
        let state = bot.state();
        let mut conversations: Vec<Value> = state.chats.iter().map(|(id, chat)| {
            let seconds = state.last_activity.get(id).map(|time| time.elapsed().as_secs());
            json!({"id":id.to_string(),"mood":chat.mood,"busy":state.busy.contains(id),"last_activity_seconds_ago":seconds,
                "activity_display":seconds.map_or_else(||t(language,"panel.state.not_reported"),|seconds|seconds_ago(language,seconds)),
                "busy_display":t(language,if state.busy.contains(id){"panel.state.busy"}else{"panel.state.idle"})})
        }).collect();
        conversations.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
        let next = crate::growth::STAGES.get(state.growth.stage + 1);
        result["next_stage"] = next.map_or(
            Value::Null,
            |stage| json!({"days":stage.min_days,"chats":stage.min_chats}),
        );
        result["name"] = json!(state.growth.name);
        result["stage"] = json!(state.growth.stage);
        result["days"] = json!(crate::growth::days(&state.growth).max(0));
        result["finished_chats"] = json!(state.growth.chats);
        result["messages"] = json!(state.growth.messages);
        result["asleep"] = json!(state.asleep);
        result["model"] = json!(state.model);
        result["thinking"] = json!(format!("{:?}", state.thinking_mode).to_lowercase());
        result["debug"] = json!(state.debug);
        result["conversations"] = json!(conversations);
        result["last_activity_seconds_ago"] = json!(state
            .last_activity
            .values()
            .map(|time| time.elapsed().as_secs())
            .min());
        result["next_transition"] = json!(if state.forced_awake_until > crate::now_unix() {
            Some(state.forced_awake_until)
        } else {
            state
                .plans
                .iter()
                .flat_map(|plan| [plan.start, plan.end])
                .filter(|time| *time > crate::now_unix())
                .min()
        });
        result["travel"] = crate::travel::now().map_or(
            Value::Null,
            |trip| json!({"place":trip.place,"reason":trip.reason,"return_day":trip.end}),
        );
        result["usage"] = json!({"period":"this_process","calls":state.metrics.calls,"input":state.metrics.input_tokens,"cached":state.metrics.cache_tokens,"output":state.metrics.output_tokens,"cost":null});
    }
    let lifecycle = result["lifecycle"].as_str().unwrap_or("unconfigured");
    let stage_key = match result["stage"].as_u64() {
        Some(0) => "panel.state.new",
        Some(1) => "panel.state.warming",
        Some(2) => "panel.state.established",
        Some(_) => "panel.state.old_hand",
        None => "panel.state.not_reported",
    };
    let number = |key: &str| {
        result[key].as_u64().map_or_else(
            || t(language, "panel.state.not_reported"),
            |value| value.to_string(),
        )
    };
    let transition = result["next_transition"].as_i64().map_or_else(
        || t(language, "panel.state.not_reported"),
        |unix| {
            let local = unix + crate::sleep::timezone_offset();
            let seconds = local.rem_euclid(86400);
            format!(
                "{} {:02}:{:02}",
                crate::memory::date_from_unix(local),
                seconds / 3600,
                seconds % 3600 / 60
            )
        },
    );
    result["display"] = json!({
        "lifecycle":t(language,&format!("panel.state.{lifecycle}")),"connection":t(language,&format!("panel.state.{connection}")),
        "model_health":t(language,"panel.state.not_reported"),"stage":t(language,stage_key),
        "name":result["name"].as_str().map_or_else(||t(language,"panel.state.no_name"),str::to_owned),
        "days":number("days"),"finished_chats":number("finished_chats"),"messages":number("messages"),
        "sleep":t(language,match result["asleep"].as_bool(){Some(true)=>"panel.state.asleep",Some(false)=>"panel.state.awake",None=>"panel.state.unknown"}),
        "debug":t(language,match result["debug"].as_bool(){Some(true)=>"panel.state.on",Some(false)=>"panel.state.off",None=>"panel.state.unknown"}),
        "thinking":result["thinking"].as_str().map_or_else(||t(language,"panel.state.not_reported"),|mode|t(language,&format!("panel.thinking.{mode}"))),
        "next_transition":transition,
        "travel":result["travel"]["place"].as_str().map_or_else(||t(language,"panel.state.not_travelling"),str::to_owned),
        "last_activity":result["last_activity_seconds_ago"].as_u64().map_or_else(||t(language,"panel.state.not_reported"),|seconds|seconds_ago(language,seconds))
    });
    result
}

fn seconds_ago(language: Language, seconds: u64) -> String {
    t(language, "panel.time.seconds_ago").replace("{seconds}", &seconds.to_string())
}

pub(super) async fn status_api(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Json<Value>> {
    let session = token(&headers)?;
    app.auth.require_admin_passive(&session)?;
    Ok(Json(snapshot(&app, lang(&query)).await))
}

pub(super) async fn overview(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    if admin(&app, &headers).is_err() {
        return Ok(Redirect::to(if app.auth.has_admin()? {
            "/login"
        } else {
            "/setup"
        })
        .into_response());
    }
    let language = lang(&query);
    let data = snapshot(&app, language).await;
    let mut page = page(language, "panel.title.overview", "overview")?;
    page.authenticated = true;
    page.description = t(language, "panel.overview.body");
    page.poll_url = format!("/api/v1/status?lang={}", language.code());
    if data["pending_restart"] == true {
        page.notice = t(language, "panel.settings.pending");
    }
    let mut health = section(language, "panel.overview.health");
    for (label, key) in [
        ("panel.label.status", "lifecycle"),
        ("panel.label.connection", "connection"),
        ("panel.label.model_health", "model_health"),
    ] {
        health.items.push(item(
            language,
            label,
            data["display"][key].as_str().unwrap_or(""),
            &format!("display.{key}"),
        ));
    }
    for (label, key) in [
        ("panel.label.provider", "provider"),
        ("panel.label.model", "model"),
        ("panel.label.spaces", "spaces"),
    ] {
        health.items.push(item(
            language,
            label,
            data[key]
                .as_str()
                .map_or_else(|| data[key].to_string(), str::to_owned),
            key,
        ));
    }
    health.items.push(Item {
        value: t(language, "panel.action.fix_connection"),
        href: format!("/setup?lang={}", language.code()),
        ..Default::default()
    });
    page.sections.push(health);
    let mut identity = section(language, "panel.overview.growth");
    identity.description = t(language, "panel.overview.growth_help");
    for (label, key) in [
        ("panel.label.name", "name"),
        ("panel.label.stage", "stage"),
        ("panel.label.days", "days"),
        ("panel.label.chats", "finished_chats"),
        ("panel.label.messages", "messages"),
    ] {
        let mut row = item(
            language,
            label,
            data["display"][key].as_str().unwrap_or(""),
            &format!("display.{key}"),
        );
        let target_key = match key {
            "days" => Some("days"),
            "finished_chats" => Some("chats"),
            _ => None,
        };
        if let Some(target_key) = target_key {
            if let Some(maximum) = data["next_stage"][target_key].as_u64() {
                row.progress_max = maximum;
                row.progress_value = data[key].as_u64().unwrap_or(0).min(maximum);
                row.detail = t(language, "panel.overview.next_threshold")
                    .replace("{target}", &maximum.to_string());
            }
        }
        identity.items.push(row);
    }
    page.sections.push(identity);
    let mut behavior = section(language, "panel.overview.sleep");
    behavior.description = t(language, "panel.overview.sleep_help");
    for (label, key) in [
        ("panel.label.sleep", "sleep"),
        ("panel.label.thinking", "thinking"),
        ("panel.label.debug", "debug"),
        ("panel.label.next_transition", "next_transition"),
        ("panel.label.travel", "travel"),
        ("panel.label.last_activity", "last_activity"),
    ] {
        behavior.items.push(item(
            language,
            label,
            data["display"][key].as_str().unwrap_or(""),
            &format!("display.{key}"),
        ));
    }
    page.sections.push(behavior);
    let mut conversations = section(language, "panel.label.active_chats");
    if let Some(entries) = data["conversations"].as_array() {
        for entry in entries {
            conversations.items.push(Item {
                label: entry["id"].as_str().unwrap_or("").into(),
                value: entry["mood"]
                    .as_str()
                    .filter(|mood| !mood.is_empty())
                    .map_or_else(|| t(language, "panel.state.not_reported"), str::to_owned),
                detail: format!(
                    "{} · {}",
                    entry["busy_display"].as_str().unwrap_or(""),
                    entry["activity_display"].as_str().unwrap_or("")
                ),
                ..Default::default()
            });
        }
    }
    if conversations.items.is_empty() {
        conversations.description = t(language, "panel.overview.no_chats");
    }
    page.sections.push(conversations);
    let mut usage = section(language, "panel.usage.title");
    usage.description = t(language, "panel.overview.usage_help");
    for (label, key) in [
        ("panel.usage.calls", "calls"),
        ("panel.usage.input", "input"),
        ("panel.usage.cached", "cached"),
        ("panel.usage.output", "output"),
    ] {
        usage.items.push(item(
            language,
            label,
            data["usage"][key].as_u64().map_or_else(
                || t(language, "panel.state.not_reported"),
                |value| value.to_string(),
            ),
            &format!("usage.{key}"),
        ));
    }
    usage.items.push(item(
        language,
        "panel.label.cost",
        t(language, "panel.state.not_reported"),
        "",
    ));
    page.sections.push(usage);
    let mut events = section(language, "panel.logs.recent");
    events.items = crate::logging::snapshot(None, 10)
        .entries
        .into_iter()
        .map(|entry| Item {
            label: format!("{} {}", entry.timestamp, entry.level),
            value: entry.message,
            ..Default::default()
        })
        .collect();
    if events.items.is_empty() {
        events.description = t(language, "panel.logs.empty");
    }
    page.sections.push(events);
    html(&page)
}

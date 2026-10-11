use super::routes::{admin, html, item, lang, page, section, t, token, WebResult};
use super::views::{Item, Language};
use super::App;
use crate::platform::capabilities::MessageVisibility;
use crate::platform::outgoing::{ConnectionSnapshot, ConnectionStatus, ConnectionWarning};
use crate::platform::types::PlatformKind;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::Arc;

pub(super) async fn snapshot(app: &App, language: Language) -> Value {
    let (lifecycle, bot) = {
        let runtime = app.runtime.lock();
        (runtime.lifecycle, runtime.bot.clone())
    };
    let connections = bot.as_ref().map_or_else(Vec::new, |bot| {
        bot.adapters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot()
    });
    let mut result = transport_snapshot(&connections);
    {
        let config = app.config.lock();
        result["lifecycle"] = json!(lifecycle);
        result["panel"] = json!("healthy");
        result["provider"] = json!(config.selected_provider());
        result["model"] = json!(config.get("MODEL"));
        result["model_health"] = json!("not_reported");
        result["pending_restart"] =
            json!(config.fields().iter().any(|field| field.pending_restart));
        result["updated_at"] = json!(crate::now_unix());
    };
    if let Some(bot) = bot {
        let state = bot.state();
        let mut conversations: Vec<Value> = state.chats.iter().map(|(id, chat)| {
            let seconds = state.last_activity.get(id).map(|time| time.elapsed().as_secs());
            let source = connections.iter().find(|connection| {
                connection.status == ConnectionStatus::Ready
                    && connection.rooms.iter().any(|room| room.room.conversation() == *id)
            });
            let label = id.thread.as_ref().map_or_else(
                || id.room.to_string(),
                |thread| format!("{} · {thread}", id.room),
            );
            json!({"id":label,"room":id.room,"thread":id.thread,
                "connection":source.map(|source|&source.connection),"platform":id.room.platform(),
                "mood":chat.mood,"busy":state.busy.contains(id),"last_activity_seconds_ago":seconds,
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
    let connection = result["connection"].as_str().unwrap_or("offline");
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
        "connections":connection_details(&connections,language),
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

/// Only reported Ready transports count as connected; lifecycle is independent.
pub(super) fn transport_snapshot(connections: &[ConnectionSnapshot]) -> Value {
    let ready = connections
        .iter()
        .filter(|connection| connection.status == ConnectionStatus::Ready);
    let mut spaces = HashSet::new();
    let mut rooms = HashSet::new();
    for connection in ready {
        for room in &connection.rooms {
            rooms.insert(&room.room.id);
            if let Some(space) = &room.room.space {
                spaces.insert(space);
            }
        }
    }
    let connection = if connections
        .iter()
        .any(|connection| connection.status == ConnectionStatus::Ready)
    {
        "connected"
    } else if connections
        .iter()
        .any(|connection| connection.status == ConnectionStatus::Connecting)
    {
        "reconnecting"
    } else {
        "offline"
    };
    json!({"connection":connection,"spaces":spaces.len(),"rooms":rooms.len(),"connections":connections})
}

fn telegram_privacy_limited(connection: &ConnectionSnapshot) -> bool {
    connection.platform == PlatformKind::Telegram
        && (connection
            .warnings
            .contains(&ConnectionWarning::TelegramPrivacyLimited)
            || connection
                .rooms
                .iter()
                .any(|room| room.capabilities.visibility == MessageVisibility::MentionsOnly))
}

fn connection_details(connections: &[ConnectionSnapshot], language: Language) -> String {
    if connections.is_empty() {
        return t(language, "panel.overview.no_connections");
    }
    let mut details = String::new();
    for (index, connection) in connections.iter().enumerate() {
        if index > 0 {
            details.push_str(" ; ");
        }
        let status = match connection.status {
            ConnectionStatus::Ready => "panel.state.connected",
            ConnectionStatus::Connecting => "panel.state.reconnecting",
            ConnectionStatus::Disconnected => "panel.state.offline",
        };
        write!(
            details,
            "{} ({}) · {} · {}: ",
            connection.connection,
            connection.platform.as_str(),
            t(language, status),
            t(language, "panel.label.account"),
        )
        .expect("writing to a String cannot fail");
        if let Some(account) = &connection.account {
            write!(details, "{} ({})", account.id, account.display_name)
                .expect("writing to a String cannot fail");
        } else {
            details.push_str(&t(language, "panel.state.not_reported"));
        }
        for room in &connection.rooms {
            write!(
                details,
                " · {}: {} · {}: {}",
                t(language, "panel.label.room"),
                json!(room.room),
                t(language, "panel.label.capabilities"),
                json!(room.capabilities),
            )
            .expect("writing to a String cannot fail");
        }
        if telegram_privacy_limited(connection) {
            write!(
                details,
                " · {}: {}",
                t(language, "panel.label.telegram_privacy"),
                t(language, "panel.overview.telegram_privacy_warning"),
            )
            .expect("writing to a String cannot fail");
        }
    }
    details
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
        ("panel.label.rooms", "rooms"),
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
    let mut connections = section(language, "panel.label.connections");
    connections.items.push(item(
        language,
        "panel.label.connections",
        data["display"]["connections"].as_str().unwrap_or(""),
        "display.connections",
    ));
    page.sections.push(connections);
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
        events.description = t(language, "panel.logs.no_recent");
    }
    page.sections.push(events);
    html(&page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{
        capabilities::{Capabilities, CountUnit, HistoryAccess},
        outgoing::RoomSnapshot,
        types::{
            Automation, ConnectionId, IdentityStability, PlatformKind, RoomKey, RoomRef, SpaceKey,
            ThreadKey, UserKey, UserRef,
        },
    };

    fn matrix_snapshot(status: ConnectionStatus) -> ConnectionSnapshot {
        let mut room =
            RoomRef::from_key(RoomKey::new(PlatformKind::Matrix, &["!room:example.org"]).unwrap());
        room.space = Some(SpaceKey::new(PlatformKind::Matrix, &["!space:example.org"]).unwrap());
        room.name = Some("<real room name>".into());
        room.thread = Some(ThreadKey::new(PlatformKind::Matrix, &["$thread:example.org"]).unwrap());
        let mut capabilities = Capabilities::unavailable();
        capabilities.send = true;
        capabilities.history = HistoryAccess::Paginated;
        capabilities.decryption_available = true;
        capabilities.output.text = 12000;
        capabilities.output.count_unit = CountUnit::EncodedBytes;
        ConnectionSnapshot {
            connection: ConnectionId::new("matrix-home"),
            platform: PlatformKind::Matrix,
            status,
            account: Some(UserRef {
                id: UserKey::new(PlatformKind::Matrix, &["@bot:example.org"]).unwrap(),
                display_name: "<real account name>".into(),
                username: None,
                automation: Automation::Bot,
                stability: IdentityStability::Persistent,
            }),
            rooms: vec![RoomSnapshot { room, capabilities }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn neutral_status_requires_ready_and_preserves_every_reported_fact() {
        let empty = transport_snapshot(&[]);
        assert_eq!(empty["connection"], "offline");
        assert_eq!(empty["connections"], json!([]));
        assert_eq!(empty["spaces"], 0);
        assert_eq!(empty["rooms"], 0);
        for (status, expected, count) in [
            (ConnectionStatus::Connecting, "reconnecting", 0),
            (ConnectionStatus::Ready, "connected", 1),
            (ConnectionStatus::Disconnected, "offline", 0),
        ] {
            let reported = matrix_snapshot(status);
            let snapshot = transport_snapshot(std::slice::from_ref(&reported));
            assert_eq!(snapshot["connection"], expected);
            assert_eq!(snapshot["spaces"], count);
            assert_eq!(snapshot["rooms"], count);
            assert_eq!(snapshot["connections"], json!([reported]));
            assert_eq!(snapshot["connections"][0]["platform"], "matrix");
            assert_eq!(snapshot["connections"][0]["connection"], "matrix-home");
            assert_eq!(
                snapshot["connections"][0]["account"]["id"],
                reported.account.as_ref().unwrap().id.as_str()
            );
            assert_eq!(
                snapshot["connections"][0]["rooms"][0]["room"],
                json!(reported.rooms[0].room)
            );
            assert_eq!(
                snapshot["connections"][0]["rooms"][0]["capabilities"],
                json!(reported.rooms[0].capabilities)
            );
            assert_eq!(
                snapshot["connections"][0]["rooms"][0]["capabilities"]["output"]["text"],
                12000
            );
            assert!(snapshot.get("cost").is_none());
        }
    }

    #[test]
    fn neutral_aggregate_deduplicates_ready_rooms_and_spaces_only() {
        let ready = matrix_snapshot(ConnectionStatus::Ready);
        let mut second = ready.clone();
        second.connection = ConnectionId::new("matrix-other");
        second.rooms[0].room.thread = None;
        let mut disconnected = matrix_snapshot(ConnectionStatus::Disconnected);
        disconnected.connection = ConnectionId::new("matrix-offline");
        disconnected.rooms[0].room.id =
            RoomKey::new(PlatformKind::Matrix, &["!offline:example.org"]).unwrap();
        disconnected.rooms[0].room.space =
            Some(SpaceKey::new(PlatformKind::Matrix, &["!offline-space:example.org"]).unwrap());
        let mut connecting = matrix_snapshot(ConnectionStatus::Connecting);
        connecting.connection = ConnectionId::new("matrix-starting");
        let snapshot =
            transport_snapshot(&[disconnected.clone(), connecting.clone(), ready, second]);
        assert_eq!(snapshot["connection"], "connected");
        assert_eq!(snapshot["spaces"], 1);
        assert_eq!(snapshot["rooms"], 1);
        assert_eq!(snapshot["connections"].as_array().unwrap().len(), 4);
        let snapshot = transport_snapshot(&[disconnected, connecting]);
        assert_eq!(snapshot["connection"], "reconnecting");
        assert_eq!(snapshot["spaces"], 0);
        assert_eq!(snapshot["rooms"], 0);
    }

    #[test]
    fn configured_registry_is_not_a_connected_transport() {
        let mut registry = crate::platform::ConnectionRegistry::default();
        let id = ConnectionId::new("slack-work");
        registry.configure(&id, PlatformKind::Slack).unwrap();
        let snapshot = transport_snapshot(&registry.snapshot());
        assert_eq!(snapshot["connection"], "reconnecting");
        assert_eq!(snapshot["connections"][0]["platform"], "slack");
        assert_eq!(snapshot["connections"][0]["connection"], "slack-work");
        assert_eq!(snapshot["connections"][0]["account"], Value::Null);
        assert_eq!(snapshot["connections"][0]["rooms"], json!([]));
        registry
            .set_status(&id, ConnectionStatus::Disconnected)
            .unwrap();
        let snapshot = transport_snapshot(&registry.snapshot());
        assert_eq!(snapshot["connection"], "offline");
        assert_eq!(snapshot["connections"][0]["status"], "Disconnected");
    }

    #[test]
    fn neutral_details_are_localized_and_keep_account_room_source_and_capabilities() {
        let reported = matrix_snapshot(ConnectionStatus::Ready);
        for language in [Language::En, Language::Tr] {
            assert_eq!(
                connection_details(&[], language),
                t(language, "panel.overview.no_connections")
            );
            let details = connection_details(std::slice::from_ref(&reported), language);
            assert!(details.contains("matrix-home (matrix)"));
            assert!(details.contains(&t(language, "panel.state.connected")));
            assert!(details.contains(&t(language, "panel.label.account")));
            assert!(details.contains(reported.account.as_ref().unwrap().id.as_str()));
            assert!(details.contains("<real account name>"));
            assert!(details.contains(&t(language, "panel.label.room")));
            assert!(details.contains(reported.rooms[0].room.id.as_str()));
            assert!(details.contains(reported.rooms[0].room.space.as_ref().unwrap().as_str()));
            assert!(details.contains(reported.rooms[0].room.thread.as_ref().unwrap().as_str()));
            assert!(details.contains(&t(language, "panel.label.capabilities")));
            assert!(details.contains(&json!(reported.rooms[0].capabilities).to_string()));
            assert!(!details.contains("discord"));
        }
    }

    #[test]
    fn slack_snapshot_and_details_preserve_reported_metadata_without_discord_extras() {
        let mut room = crate::platform::slack::events::room("T123", "C456").unwrap();
        room.name = Some("approved Slack room".into());
        room.thread = Some(
            ThreadKey::new(PlatformKind::Slack, &["T123", "C456", "1700000000.000123"]).unwrap(),
        );
        let mut account = crate::platform::slack::events::user("T123", "U789").unwrap();
        account.display_name = "reported Slack bot".into();
        account.automation = Automation::Bot;
        let mut capabilities = Capabilities::unavailable();
        capabilities.send = true;
        capabilities.history = HistoryAccess::Paginated;
        capabilities.visibility = MessageVisibility::Ordinary;
        let reported = ConnectionSnapshot {
            connection: ConnectionId::new("slack-work"),
            platform: PlatformKind::Slack,
            status: ConnectionStatus::Ready,
            account: Some(account),
            rooms: vec![RoomSnapshot { room, capabilities }],
            warnings: Vec::new(),
        };
        let snapshot = transport_snapshot(std::slice::from_ref(&reported));
        assert_eq!(snapshot["connection"], "connected");
        assert_eq!(snapshot["spaces"], 1);
        assert_eq!(snapshot["rooms"], 1);
        assert_eq!(snapshot["connections"], json!([reported]));
        assert_eq!(
            snapshot["connections"][0]["account"]["id"],
            "slack:user:T123:U789"
        );
        assert_eq!(
            snapshot["connections"][0]["rooms"][0]["room"]["id"],
            "slack:room:T123:C456"
        );
        assert!(snapshot.get("guilds").is_none());
        assert!(snapshot.get("cost").is_none());
        for language in [Language::En, Language::Tr] {
            let details = connection_details(std::slice::from_ref(&reported), language);
            assert!(details.contains("slack-work (slack)"));
            assert!(details.contains("reported Slack bot"));
            assert!(details.contains("slack:user:T123:U789"));
            assert!(details.contains("slack:room:T123:C456"));
            assert!(details.contains("slack:space:T123"));
            assert!(details.contains(reported.rooms[0].room.thread.as_ref().unwrap().as_str()));
            assert!(details.contains(&json!(reported.rooms[0].capabilities).to_string()));
            assert!(!details.contains("discord"));
            assert!(!details.contains(&t(language, "panel.overview.telegram_privacy_warning")));
        }
    }

    #[test]
    fn telegram_privacy_warning_uses_reported_visibility_and_account_metadata_only() {
        let mut connection = matrix_snapshot(ConnectionStatus::Ready);
        connection.connection = ConnectionId::new("telegram");
        connection.platform = PlatformKind::Telegram;
        connection.account.as_mut().unwrap().id =
            UserKey::new(PlatformKind::Telegram, &["99"]).unwrap();
        connection.rooms[0].room =
            RoomRef::from_key(RoomKey::new(PlatformKind::Telegram, &["-100123"]).unwrap());
        assert!(telegram_privacy_limited(&connection));
        for language in [Language::En, Language::Tr] {
            let warning = t(language, "panel.overview.telegram_privacy_warning");
            assert!(!warning.starts_with("panel."));
            let details = connection_details(std::slice::from_ref(&connection), language);
            assert!(details.contains(&warning));
            assert!(details.contains(&t(language, "panel.label.telegram_privacy")));
            assert!(details.contains("telegram:room:-100123"));
        }
        connection.rooms[0].capabilities.visibility = MessageVisibility::Ordinary;
        assert!(!telegram_privacy_limited(&connection));
        assert!(
            !connection_details(std::slice::from_ref(&connection), Language::En)
                .contains(&t(Language::En, "panel.overview.telegram_privacy_warning"))
        );
        connection
            .warnings
            .push(ConnectionWarning::TelegramPrivacyLimited);
        assert!(telegram_privacy_limited(&connection));
        connection.rooms.clear();
        assert!(telegram_privacy_limited(&connection));
        let snapshot = transport_snapshot(std::slice::from_ref(&connection));
        assert_eq!(
            snapshot["connections"][0]["warnings"],
            json!([ConnectionWarning::TelegramPrivacyLimited])
        );
        for language in [Language::En, Language::Tr] {
            assert!(
                connection_details(std::slice::from_ref(&connection), language)
                    .contains(&t(language, "panel.overview.telegram_privacy_warning"))
            );
        }
        connection.platform = PlatformKind::Discord;
        assert!(!telegram_privacy_limited(&connection));
        assert!(
            !connection_details(std::slice::from_ref(&connection), Language::En)
                .contains(&t(Language::En, "panel.overview.telegram_privacy_warning"))
        );
    }
}

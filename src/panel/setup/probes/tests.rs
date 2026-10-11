use super::*;
use serde_json::json;

fn telegram_body() -> Value {
    json!({"ok": true, "result": {
        "id": 123456789, "is_bot": true, "first_name": "Group bot", "username": "GroupTestBot"
    }})
}

fn slack_auth() -> Value {
    json!({"ok": true, "user_id": "U123ABC", "bot_id": "B456DEF", "team_id": "T789GHI", "user": "group-bot"})
}

fn slack_connection() -> Value {
    json!({"ok": true, "url": "wss://wss-primary.slack.com/link/?ticket=private-socket-ticket"})
}

#[test]
fn discord_uses_authenticated_application_id_for_invite() {
    let application =
        discord_application(&json!({"id": "987654321", "name": "Verified bot"})).unwrap();
    assert_eq!(application.platform, PlatformKind::Discord);
    assert_eq!(application.id, "987654321");
    assert_eq!(application.name, "Verified bot");
    assert_eq!(application.invite_url, "https://discord.com/oauth2/authorize?client_id=987654321&permissions=67226688&scope=bot%20applications.commands");
    assert_eq!(application.workspace, None);
    assert_eq!(application.privacy_mode, None);
    for body in [
        json!({}),
        json!({"id": 987654321}),
        json!({"id": "0"}),
        json!({"id": "-1"}),
        json!({"id": "invalid_auth"}),
        json!({"id": "18446744073709551616"}),
    ] {
        assert!(discord_application(&body).is_err());
    }
}

#[test]
fn telegram_privacy_flags_are_explicit_and_unknown_is_not_assumed() {
    let mut body = telegram_body();
    let application = telegram_application(&body).unwrap();
    assert_eq!(application.platform, PlatformKind::Telegram);
    assert_eq!(application.id, "123456789");
    assert_eq!(
        application.invite_url,
        "https://t.me/GroupTestBot?startgroup=true"
    );
    assert_eq!(application.privacy_mode, None);
    body["result"]["can_read_all_group_messages"] = json!(true);
    assert_eq!(
        telegram_application(&body).unwrap().privacy_mode,
        Some(false)
    );
    body["result"]["can_read_all_group_messages"] = json!(false);
    assert_eq!(
        telegram_application(&body).unwrap().privacy_mode,
        Some(true)
    );
    for flag in [json!(null), json!("true"), json!(1), json!({})] {
        body["result"]["can_read_all_group_messages"] = flag;
        assert!(telegram_application(&body).is_err());
    }
}

#[test]
fn telegram_rejects_bad_credentials_and_malformed_identity() {
    for body in [
        json!({"ok": false, "description": "secret-token"}),
        json!({"ok": "true", "result": {}}),
        json!({"ok": true}),
        json!([]),
    ] {
        let error = telegram_application(&body).err().unwrap().to_string();
        assert_eq!(error, INVALID_RESPONSE);
        assert!(!error.contains("secret-token"));
    }
    for (key, value) in [
        ("is_bot", json!(false)),
        ("is_bot", json!("true")),
        ("id", json!("123456789")),
        ("id", json!(0)),
        ("id", json!(-1)),
        ("id", json!(1.5)),
        ("username", json!(null)),
        ("username", json!("bad/OtherBot")),
        ("username", json!("NotABot?startgroup=evil")),
        ("username", json!("plain_user")),
        ("first_name", json!("")),
        ("first_name", json!(null)),
    ] {
        let mut body = telegram_body();
        body["result"][key] = value;
        assert!(
            telegram_application(&body).is_err(),
            "accepted malformed {key}"
        );
    }
}

#[test]
fn slack_requires_real_bot_workspace_and_app_socket_response() {
    let auth = slack_auth();
    let application = slack_application(&auth, &slack_connection()).unwrap();
    assert_eq!(application.platform, PlatformKind::Slack);
    assert_eq!(application.id, "U123ABC");
    assert_eq!(application.workspace.as_deref(), Some("T789GHI"));
    assert!(application.invite_url.is_empty());
    let published = serde_json::to_string(&application).unwrap();
    assert!(!published.contains("private-socket-ticket"));
    assert!(!published.contains("wss:"));
    for key in ["team_id", "user_id", "bot_id"] {
        let mut malformed = auth.clone();
        malformed.as_object_mut().unwrap().remove(key);
        assert!(slack_application(&malformed, &slack_connection()).is_err());
        malformed[key] = json!("invalid");
        assert!(slack_application(&malformed, &slack_connection()).is_err());
    }
    for response in [
        json!({"ok": false, "error": "invalid_auth"}),
        json!({"ok": true}),
        json!({"ok": "true", "url": "wss://wss-primary.slack.com"}),
    ] {
        assert!(slack_application(&auth, &response).is_err());
    }
    assert!(slack_application(
        &json!({"ok": false, "error": "invalid_auth"}),
        &slack_connection()
    )
    .is_err());
}

#[test]
fn slack_rejects_reported_workspace_and_account_mismatch() {
    for (key, reported) in [
        ("team_id", json!("TOTHER")),
        ("user_id", json!("UOTHER")),
        ("bot_id", json!("BOTHER")),
        ("team_id", json!(null)),
    ] {
        let mut connection = slack_connection();
        connection[key] = reported;
        assert_eq!(
            slack_application(&slack_auth(), &connection)
                .err()
                .unwrap()
                .to_string(),
            IDENTITY_MISMATCH
        );
    }
    let mut connection = slack_connection();
    connection["team_id"] = json!("T789GHI");
    connection["user_id"] = json!("U123ABC");
    connection["bot_id"] = json!("B456DEF");
    assert!(slack_application(&slack_auth(), &connection).is_ok());
}

#[test]
fn slack_rejects_nonsecure_or_malformed_socket_urls_without_disclosing_them() {
    for socket in [
        "",
        "not a URL",
        "ws://wss-primary.slack.com/?ticket=private-ticket",
        "https://wss-primary.slack.com/?ticket=private-ticket",
        "wss://evilslack.com/?ticket=private-ticket",
        "wss://slack.com.evil.example/?ticket=private-ticket",
        "wss://secret@wss-primary.slack.com/?ticket=private-ticket",
        "wss://wss-primary.slack.com/#private-ticket",
    ] {
        let mut connection = slack_connection();
        connection["url"] = json!(socket);
        let error = slack_application(&slack_auth(), &connection)
            .err()
            .unwrap()
            .to_string();
        assert_eq!(error, INVALID_RESPONSE);
        assert!(!error.contains("private-ticket"));
        assert!(!error.contains(socket) || socket.is_empty());
    }
}

#[test]
fn request_construction_uses_fixed_endpoints_and_separate_authorization() {
    let client = client().unwrap();
    let discord = discord_request(&client, "discord-private-token").unwrap();
    assert_eq!(discord.method(), reqwest::Method::GET);
    assert_eq!(
        discord.url().as_str(),
        "https://discord.com/api/v10/oauth2/applications/@me"
    );
    let header = &discord.headers()[reqwest::header::AUTHORIZATION];
    assert_eq!(header.to_str().unwrap(), "Bot discord-private-token");
    assert!(header.is_sensitive());
    let telegram = telegram_request(&client, "123456789:private_token").unwrap();
    assert_eq!(telegram.method(), reqwest::Method::GET);
    assert_eq!(telegram.url().host_str(), Some("api.telegram.org"));
    assert_eq!(telegram.url().path(), "/bot123456789:private_token/getMe");
    assert_eq!(telegram.url().query(), None);
    for (key, token, endpoint) in [
        (
            "SLACK_BOT_TOKEN",
            "xoxb-private-bot",
            "https://slack.com/api/auth.test",
        ),
        (
            "SLACK_APP_TOKEN",
            "xapp-private-app",
            "https://slack.com/api/apps.connections.open",
        ),
    ] {
        let request = slack_request(&client, key, token).unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(request.url().as_str(), endpoint);
        assert_eq!(
            request.headers()[reqwest::header::AUTHORIZATION]
                .to_str()
                .unwrap(),
            format!("Bearer {token}")
        );
        assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
        assert!(request.body().is_none());
    }
}

#[test]
fn wrong_token_types_and_path_injection_fail_before_requests() {
    let client = client().unwrap();
    for token in [
        "",
        "secret\r\nInjected: header",
        "secret\0token",
        "secret token",
    ] {
        assert!(discord_request(&client, token).is_err());
    }
    for token in [
        "",
        "xoxb-wrong",
        "0:secret",
        "123:secret/getMe",
        "123:secret?redirect=evil",
        "123:secret#fragment",
        "123:secret:extra",
        "abc:secret",
        "123:",
    ] {
        let error = telegram_request(&client, token).err().unwrap().to_string();
        assert!(!error.contains("redirect=evil"));
    }
    for (key, token) in [
        ("SLACK_APP_TOKEN", "xoxb-wrong"),
        ("SLACK_BOT_TOKEN", "xapp-wrong"),
        ("SLACK_BOT_TOKEN", "xoxp-user"),
        ("SLACK_APP_TOKEN", "xapp-"),
        ("SLACK_BOT_TOKEN", ""),
        ("SLACK_BOT_TOKEN", "xoxb-secret\r\nInjected: value"),
    ] {
        assert!(slack_request(&client, key, token).is_err());
    }
}

#[cfg(feature = "matrix")]
fn matrix_values() -> HashMap<String, String> {
    HashMap::from([
        (
            "MATRIX_HOMESERVER".into(),
            "https://matrix.example/base/".into(),
        ),
        ("MATRIX_USER".into(), "@groupbot:example.org".into()),
        ("MATRIX_ACCESS_TOKEN".into(), "private-matrix-token".into()),
    ])
}

#[cfg(feature = "matrix")]
#[test]
fn matrix_request_validates_homeserver_and_uses_bearer_whoami() {
    let client = client().unwrap();
    let values = matrix_values();
    let request = matrix_request(&client, &values).unwrap();
    assert_eq!(
        request.url().as_str(),
        "https://matrix.example/base/_matrix/client/v3/account/whoami"
    );
    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(
        request.headers()[reqwest::header::AUTHORIZATION]
            .to_str()
            .unwrap(),
        "Bearer private-matrix-token"
    );
    assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
    for homeserver in [
        "",
        "ftp://matrix.example",
        "https://secret@matrix.example",
        "https://matrix.example?token=private",
        "https://matrix.example/#fragment",
    ] {
        let mut values = values.clone();
        values.insert("MATRIX_HOMESERVER".into(), homeserver.into());
        let error = matrix_request(&client, &values).err().unwrap().to_string();
        assert!(!error.contains("secret@"));
        assert!(!error.contains("token=private"));
    }
    for key in ["MATRIX_USER", "MATRIX_ACCESS_TOKEN"] {
        let mut values = values.clone();
        values.remove(key);
        assert!(matrix_request(&client, &values).is_err());
    }
}

#[cfg(feature = "matrix")]
#[test]
fn matrix_whoami_requires_exact_full_user_not_only_localpart() {
    let expected = "@groupbot:example.org";
    let application = matrix_application(
        &json!({"user_id": expected, "device_id": "DEVICE"}),
        expected,
    )
    .unwrap();
    assert_eq!(application.id, expected);
    assert_eq!(application.platform, PlatformKind::Matrix);
    assert!(application.invite_url.is_empty());
    for user in ["@other:example.org", "@groupbot:other.example"] {
        assert_eq!(
            matrix_application(&json!({"user_id": user}), expected)
                .err()
                .unwrap()
                .to_string(),
            IDENTITY_MISMATCH
        );
    }
    for body in [
        json!({"errcode": "M_UNKNOWN_TOKEN", "error": "private-matrix-token"}),
        json!({}),
        json!({"user_id": "groupbot"}),
        json!({"user_id": 42}),
        json!({"user_id": "@bad user:example.org"}),
        json!({"user_id": " @groupbot:example.org "}),
    ] {
        let error = matrix_application(&body, expected)
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("private-matrix-token"));
    }
}

#[cfg(not(feature = "matrix"))]
#[tokio::test]
async fn matrix_disabled_is_rejected_without_request_or_false_success() {
    let values = HashMap::from([
        (
            "MATRIX_HOMESERVER".into(),
            "https://unreachable.invalid".into(),
        ),
        ("MATRIX_ACCESS_TOKEN".into(), "private-matrix-token".into()),
        ("MATRIX_USER".into(), "@groupbot:example.org".into()),
    ]);
    let error = test(PlatformKind::Matrix, &values)
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(error, "panel.config.matrix_feature_required");
    assert!(!error.contains("private-matrix-token"));
}

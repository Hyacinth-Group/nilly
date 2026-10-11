use super::*;
use serde_json::json;

fn config(values: &[(&str, &str)]) -> (tempfile::TempDir, crate::config::Config) {
    let directory = tempfile::tempdir().unwrap();
    let environment = values
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let config = crate::config::Config::load(directory.path().join(".env"), &environment).unwrap();
    (directory, config)
}

fn application(platform: PlatformKind) -> Application {
    Application {
        id: "verified-identity".into(),
        name: "Verified bot".into(),
        invite_url: String::new(),
        platform,
        privacy_mode: None,
        workspace: None,
    }
}

#[test]
fn draft_defaults_to_telegram_and_detects_first_configured_platform() {
    for (values, expected) in [
        (vec![], PlatformKind::Telegram),
        (
            vec![("DISCORD_TOKEN", "discord-token")],
            PlatformKind::Discord,
        ),
        (
            vec![("TELEGRAM_TOKEN", "telegram-token")],
            PlatformKind::Telegram,
        ),
        (vec![("SLACK_APP_TOKEN", "app-token")], PlatformKind::Slack),
        (vec![("SLACK_BOT_TOKEN", "bot-token")], PlatformKind::Slack),
        (
            vec![("MATRIX_HOMESERVER", "https://matrix.example")],
            PlatformKind::Matrix,
        ),
        (
            vec![("MATRIX_ACCESS_TOKEN", "matrix-token")],
            PlatformKind::Matrix,
        ),
        (
            vec![
                ("DISCORD_TOKEN", "discord-token"),
                ("TELEGRAM_TOKEN", "telegram-token"),
                ("SLACK_APP_TOKEN", "app-token"),
            ],
            PlatformKind::Discord,
        ),
        (
            vec![
                ("TELEGRAM_TOKEN", "telegram-token"),
                ("SLACK_APP_TOKEN", "app-token"),
                ("MATRIX_HOMESERVER", "https://matrix.example"),
            ],
            PlatformKind::Telegram,
        ),
        (
            vec![
                ("SLACK_APP_TOKEN", "app-token"),
                ("MATRIX_HOMESERVER", "https://matrix.example"),
            ],
            PlatformKind::Slack,
        ),
    ] {
        let (_directory, config) = config(&values);
        let draft = Draft::new(&config);
        assert_eq!(draft.selected_platform, expected);
        assert_eq!(draft.platform_attempt, 0);
        assert_eq!(draft.step(), 2);
        assert_eq!(draft.revision, config.revision());
        assert!(draft.expires > Instant::now());
    }
}

#[test]
fn recreated_draft_has_distinct_identity_even_when_attempts_match() {
    let (_directory, config) = config(&[]);
    let mut expired = Draft::new(&config);
    expired.platform_attempt = 1;
    expired.expires = Instant::now() - Duration::from_secs(1);
    let mut replacement = Draft::new(&config);
    replacement.platform_attempt = 1;
    assert_eq!(expired.selected_platform, replacement.selected_platform);
    assert_eq!(expired.platform_attempt, replacement.platform_attempt);
    assert_ne!(expired.platform_nonce, replacement.platform_nonce);
    let nonce = replacement.platform_nonce;
    replacement.platform_attempt += 1;
    replacement.selected_platform = PlatformKind::Slack;
    assert_eq!(replacement.platform_nonce, nonce);
}

#[test]
fn draft_preserves_other_platform_credentials_and_access_controls() {
    let values = [
        ("DISCORD_TOKEN", "discord-secret"),
        ("OWNER_ID", "123"),
        ("GUILD_ID", "456"),
        ("CHANNELS", "789"),
        ("DEBUG_CHANNEL", "111"),
        ("NEWS_CHANNEL", "222"),
        ("TELEGRAM_TOKEN", "telegram-secret"),
        ("TELEGRAM_ROOMS", "-100123"),
        ("SLACK_APP_TOKEN", "app-secret"),
        ("SLACK_BOT_TOKEN", "bot-secret"),
        ("SLACK_CHANNELS", "C123"),
        ("SLACK_HISTORY_TIER", "restricted"),
        ("SLACK_TEST_CHANNEL", "C456"),
        ("MATRIX_HOMESERVER", "https://matrix.example"),
        ("MATRIX_USER", "@bot:example.org"),
        ("MATRIX_ACCESS_TOKEN", "matrix-secret"),
        ("MATRIX_PASSWORD", "matrix-password"),
        ("MATRIX_ROOMS", "!room:example.org"),
        ("OWNER_KEY", "telegram:user:123"),
        ("PROVIDER", "mistral"),
        ("OPENROUTER_KEY", "router-secret"),
        ("MISTRAL_KEY", "mistral-secret"),
        ("MODEL", "mistral-small-latest"),
        ("BOT_LANG", "tr"),
        ("TZ_OFFSET", "3"),
        ("NEWS_RSS", "https://news.example/rss"),
    ];
    let (_directory, config) = config(&values);
    let mut draft = Draft::new(&config);
    draft.selected_platform = PlatformKind::Slack;
    draft
        .values
        .insert("SLACK_BOT_TOKEN".into(), "replacement-secret".into());
    for (key, _) in values {
        if key != "SLACK_BOT_TOKEN" {
            assert_eq!(
                draft.values.get(key).map(String::as_str),
                Some(config.get(key)),
                "lost {key}"
            );
        }
    }
}

#[test]
fn draft_progression_requires_verified_selected_platform_and_all_steps() {
    let (_directory, config) = config(&[]);
    for platform in [
        PlatformKind::Discord,
        PlatformKind::Telegram,
        PlatformKind::Slack,
        PlatformKind::Matrix,
    ] {
        let mut draft = Draft::new(&config);
        draft.selected_platform = platform;
        assert_eq!(draft.step(), 2);
        draft.application = Some(application(platform));
        assert_eq!(draft.step(), 3);
        draft.provider_verified = true;
        assert_eq!(draft.step(), 4);
        draft.fit_completed = true;
        assert_eq!(draft.step(), 5);
        draft.selected_platform = if platform == PlatformKind::Telegram {
            PlatformKind::Slack
        } else {
            PlatformKind::Telegram
        };
        assert_eq!(
            draft.step(),
            2,
            "a previous platform's verification must not advance this choice"
        );
        draft.application = None;
        assert_eq!(draft.step(), 2);
    }
}

#[test]
fn platform_parser_accepts_chat_platforms_not_terminal_or_unknown() {
    for platform in [
        PlatformKind::Discord,
        PlatformKind::Telegram,
        PlatformKind::Slack,
        PlatformKind::Matrix,
    ] {
        assert_eq!(parse_platform(platform.as_str()).unwrap(), platform);
    }
    for raw in [
        "",
        "terminal",
        "unknown",
        "Discord",
        " telegram",
        "telegram\n",
    ] {
        assert!(parse_platform(raw).is_err());
    }
}

#[tokio::test]
async fn bounded_response_rejects_redirects_large_and_malformed_bodies() {
    for status in [302, 401, 403, 429, 500] {
        let response = axum::http::Response::builder()
            .status(status)
            .body("private-credential-response")
            .unwrap();
        let error = bounded_json(response.into())
            .await
            .err()
            .unwrap()
            .to_string();
        assert_eq!(error, "panel.setup.probe_rejected");
        assert!(!error.contains("private-credential-response"));
    }
    let response = axum::http::Response::builder()
        .status(200)
        .body("x".repeat(256 * 1024 + 1))
        .unwrap();
    assert_eq!(
        bounded_json(response.into())
            .await
            .err()
            .unwrap()
            .to_string(),
        "panel.setup.probe_response_too_large"
    );
    let response = axum::http::Response::builder()
        .status(200)
        .body("{\"error\": private-credential-response}")
        .unwrap();
    let error = bounded_json(response.into())
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(error, "panel.setup.probe_response_invalid");
    assert!(!error.contains("private-credential-response"));
    let response = axum::http::Response::builder()
        .status(200)
        .body("{\"ok\":true}")
        .unwrap();
    assert_eq!(
        bounded_json(response.into()).await.unwrap(),
        json!({"ok": true})
    );
}

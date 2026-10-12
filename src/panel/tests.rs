use super::*;
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

pub(super) async fn fixture() -> (Arc<App>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let config = config::Config::load(directory.path().join(".env"), &HashMap::new()).unwrap();
    let auth = auth::AuthService::isolated(&directory.path().join("test.redb"))
        .await
        .unwrap();
    let (start, _receiver) = mpsc::channel(1);
    let (_sender, stop) = watch::channel(false);
    (
        Arc::new(App {
            auth,
            origin: auth::OriginPolicy::new("http://localhost:8787").unwrap(),
            secure: false,
            config: Mutex::new(config),
            runtime: Mutex::new(Runtime {
                lifecycle: Lifecycle::Unconfigured,
                attempted: false,
                bot: None,
            }),
            drafts: Mutex::new(HashMap::new()),
            start,
            stop,
            tests: Arc::new(tokio::sync::Semaphore::new(2)),
            streams: Arc::new(tokio::sync::Semaphore::new(8)),
            jobs: Arc::new(actions::JobStore::new()),
        }),
        directory,
    )
}

pub(super) async fn owner(app: &App) -> auth::SessionIssue {
    let peer = "127.0.0.1".parse().unwrap();
    let claim = app.auth.renew_claim().unwrap().unwrap();
    let preauth = app.auth.preauth(peer).unwrap();
    let setup = app
        .auth
        .claim(&claim, &preauth.token, &preauth.info.csrf, peer)
        .unwrap();
    app.auth
        .create_admin(
            "a private administrator password".into(),
            &setup.token,
            &setup.info.csrf,
            peer,
        )
        .await
        .unwrap()
}

pub(super) fn request(method: &str, path: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost:8787");
    if method != "GET" {
        request = request
            .header(header::ORIGIN, "http://localhost:8787")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, format!("{}={cookie}", auth::COOKIE_NAME));
    }
    let mut request = request.body(Body::from(body.to_owned())).unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:50000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    request
}

#[tokio::test]
async fn every_private_route_requires_administrator_and_liveness_is_public() {
    let (app, _directory) = fixture().await;
    let router = routes::router(app);
    for path in [
        "/settings",
        "/setup/slack-app-manifest.yml",
        "/logs",
        "/account",
        "/help",
        "/memory",
        "/personality",
        "/actions",
        "/usage",
        "/api/v1/memory/people",
        "/api/v1/personality",
        "/api/v1/actions",
        "/api/v1/connections",
        "/api/v1/usage",
        "/api/v1/status",
        "/api/v1/settings",
        "/api/v1/logs",
        "/api/v1/logs/stream",
    ] {
        let response = router
            .clone()
            .oneshot(request("GET", path, None, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    let response = router
        .clone()
        .oneshot(request("GET", "/healthz", None, ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({"status":"ok"})
    );
    let response = router
        .oneshot(request("GET", "/assets/htmx-2.0.4.min.js", None, ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
}

#[tokio::test]
async fn browser_mutations_reject_csrf_and_wrong_password_as_html() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            "/ui/auth/logout",
            Some(&admin.token),
            "csrf=wrong",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    app.auth.require_admin(&admin.token).unwrap();
    let preauth = app.auth.preauth("127.0.0.1".parse().unwrap()).unwrap();
    let response = router
        .oneshot(request(
            "POST",
            "/ui/auth/login?lang=tr",
            Some(&preauth.token),
            &format!("csrf={}&password=wrong", preauth.info.csrf),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        body.contains(views::text(views::Language::Tr, "panel.auth_invalid_credentials").unwrap())
    );
    assert!(body.contains("/login?lang=tr"));
}

#[tokio::test]
async fn dns_rebinding_and_cross_origin_posts_are_rejected_with_security_headers() {
    let (app, _directory) = fixture().await;
    let router = routes::router(app);
    let mut malicious = request("GET", "/healthz", None, "");
    malicious
        .headers_mut()
        .insert(header::HOST, "attacker.example:8787".parse().unwrap());
    let response = router.clone().oneshot(malicious).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let mut malicious = request("POST", "/api/v1/setup/claim", None, "{}");
    malicious
        .headers_mut()
        .insert(header::ORIGIN, "http://attacker.example".parse().unwrap());
    let response = router.oneshot(malicious).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response.headers().contains_key("content-security-policy"));
    assert_eq!(response.headers()["referrer-policy"], "same-origin");
}

#[tokio::test]
async fn setup_claim_rotates_cookie_and_cannot_be_replayed() {
    let (app, _directory) = fixture().await;
    let claim = app.auth.renew_claim().unwrap().unwrap();
    let preauth = app.auth.preauth("127.0.0.1".parse().unwrap()).unwrap();
    let body = format!("csrf={}&token={claim}", preauth.info.csrf);
    let router = routes::router(app);
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            "/ui/setup/claim",
            Some(&preauth.token),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(!cookie.contains(&preauth.token));
    let replay = router
        .oneshot(request(
            "POST",
            "/ui/setup/claim",
            Some(&preauth.token),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn panel_alive_is_not_platform_or_model_readiness() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let response = routes::router(app)
        .oneshot(request("GET", "/api/v1/status", Some(&admin.token), ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 32 * 1024).await.unwrap()).unwrap();
    assert_eq!(value["panel"], "healthy");
    assert_eq!(value["connection"], "offline");
    assert_eq!(value["model_health"], "not_reported");
    assert_eq!(value["lifecycle"], "unconfigured");
}

#[test]
fn setup_state_machine_has_honest_transitions() {
    let mut state = Lifecycle::Unconfigured;
    assert!(state.start().is_ok());
    assert_eq!(state, Lifecycle::ConfiguredStarting);
    assert!(state.start().is_err());
    state = Lifecycle::Running;
    assert!(state.start().is_err());
    state = Lifecycle::Stopping;
    assert!(state.start().is_err());
}

fn ready_draft(app: &App, session: &str) {
    use sha2::Digest;
    let config = app.config.lock();
    let mut draft = setup::Draft::new(&config);
    draft
        .values
        .insert("DISCORD_TOKEN".into(), "private-discord-token".into());
    draft
        .values
        .insert("OPENROUTER_KEY".into(), "private-provider-key".into());
    draft.values.insert("BOT_LANG".into(), "tr".into());
    draft.values.insert("TZ_OFFSET".into(), "5.5".into());
    draft.selected_platform = crate::platform::types::PlatformKind::Discord;
    draft.application = Some(setup::Application {
        id: "42".into(),
        name: "Verified test application".into(),
        invite_url: "https://discord.com/oauth2/authorize?client_id=42&permissions=67226688&scope=bot%20applications.commands".into(),
        platform: draft.selected_platform,
        privacy_mode: None,
        workspace: None,
    });
    draft.provider_verified = true;
    draft.fit_completed = true;
    app.drafts.lock().insert(
        format!("{:x}", sha2::Sha256::digest(session.as_bytes())),
        draft,
    );
}

#[tokio::test]
async fn wizard_cannot_skip_tests_and_finish_checkbox_submits_correct_value() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    let response = router
        .clone()
        .oneshot(request("GET", "/setup?step=5", Some(&admin.token), ""))
        .await
        .unwrap();
    let body = String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("platform-form"));
    assert!(!body.contains("complete-form"));
    ready_draft(&app, &admin.token);
    let response = router
        .oneshot(request("GET", "/setup?step=5", Some(&admin.token), ""))
        .await
        .unwrap();
    let body = String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("name=\"access_confirmed\" value=\"on\""));
    assert!(body.contains("href=\"https://discord.com/oauth2/authorize?client_id=42"));
    assert!(!body.contains("private-discord-token"));
    assert!(!body.contains("private-provider-key"));
}

#[tokio::test]
async fn wizard_finishes_into_first_runtime_snapshot_without_environment_mutation() {
    let (mut app, directory) = fixture().await;
    let (start, mut receiver) = mpsc::channel(1);
    Arc::get_mut(&mut app).unwrap().start = start;
    let admin = owner(&app).await;
    ready_draft(&app, &admin.token);
    let original_language = std::env::var("BOT_LANG");
    let router = routes::router(app.clone());
    let body = format!("csrf={}&access_confirmed=on", admin.info.csrf);
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            "/ui/setup/complete",
            Some(&admin.token),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let runtime_config = receiver.recv().await.unwrap();
    assert_eq!(runtime_config.get("BOT_LANG"), "tr");
    assert_eq!(runtime_config.get("TZ_OFFSET"), "5.5");
    assert_eq!(std::env::var("BOT_LANG"), original_language);
    assert!(directory.path().join(".env").is_file());
    assert_eq!(app.runtime.lock().lifecycle, Lifecycle::ConfiguredStarting);
    assert!(!app
        .config
        .lock()
        .fields()
        .iter()
        .any(|field| field.pending_restart));
    let before = std::fs::read(directory.path().join(".env")).unwrap();
    let duplicate = router
        .oneshot(request(
            "POST",
            "/ui/setup/complete",
            Some(&admin.token),
            &body,
        ))
        .await
        .unwrap();
    assert!(duplicate.status().is_client_error());
    assert_eq!(
        std::fs::read(directory.path().join(".env")).unwrap(),
        before
    );
}

#[tokio::test]
async fn rejected_runtime_restart_never_writes_configuration() {
    let (app, directory) = fixture().await;
    let admin = owner(&app).await;
    ready_draft(&app, &admin.token);
    {
        let mut runtime = app.runtime.lock();
        runtime.lifecycle = Lifecycle::ConfigurationError;
        runtime.attempted = true;
    }
    let response = routes::router(app)
        .oneshot(request(
            "POST",
            "/ui/setup/complete",
            Some(&admin.token),
            &format!("csrf={}&access_confirmed=on", admin.info.csrf),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!directory.path().join(".env").exists());
}

fn wizard_key(session: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(session.as_bytes()))
}

fn application(platform: crate::platform::types::PlatformKind) -> setup::Application {
    use crate::platform::types::PlatformKind;
    setup::Application {
        id: if platform == PlatformKind::Matrix {
            "@wizardbot:example.org"
        } else {
            "42"
        }
        .into(),
        name: "Verified wizard bot".into(),
        invite_url: match platform {
            PlatformKind::Discord => "https://discord.com/oauth2/authorize?client_id=42",
            PlatformKind::Telegram => "https://t.me/wizardbot?startgroup=true",
            _ => "",
        }
        .into(),
        platform,
        privacy_mode: None,
        workspace: (platform == PlatformKind::Slack).then(|| "T12345678".into()),
    }
}

fn platform_draft(app: &App, session: &str, platform: crate::platform::types::PlatformKind) {
    ready_draft(app, session);
    let mut drafts = app.drafts.lock();
    let draft = drafts.get_mut(&wizard_key(session)).unwrap();
    draft.selected_platform = platform;
    draft.application = Some(application(platform));
    for &key in routes::platform_credentials(platform) {
        draft.values.insert(
            key.into(),
            match key {
                "MATRIX_HOMESERVER" => "https://matrix.example.org",
                "MATRIX_USER" => "@wizardbot:example.org",
                "DISCORD_TOKEN" => "private-discord-token",
                _ => "private-platform-credential",
            }
            .into(),
        );
    }
}

async fn response_text(response: axum::response::Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn wizard_cards_select_natively_and_never_echo_credentials() {
    use crate::platform::types::PlatformKind;
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    let response = router
        .clone()
        .oneshot(request("GET", "/setup?step=2", Some(&admin.token), ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    for platform in ["telegram", "discord", "slack", "matrix"] {
        assert!(body.contains(&format!(
            "href=\"/setup?step=2&#38;platform={platform}&#38;lang=en\""
        )));
    }
    assert!(body.contains("Recommended"));
    assert!(body.contains("name=\"TELEGRAM_TOKEN\""));
    for platform in [
        PlatformKind::Discord,
        PlatformKind::Telegram,
        PlatformKind::Slack,
        PlatformKind::Matrix,
    ] {
        platform_draft(&app, &admin.token, platform);
        let response = router
            .clone()
            .oneshot(request(
                "GET",
                &format!("/setup?step=2&platform={}", platform.as_str()),
                Some(&admin.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        for key in routes::platform_credentials(platform) {
            assert!(body.contains(&format!("name=\"{key}\"")));
        }
        assert!(!body.contains("private-platform-credential"));
        assert!(!body.contains("private-discord-token"));
        assert!(!body.contains("private-provider-key"));
        assert!(body.contains(&format!(
            "name=\"platform\" value=\"{}\"",
            platform.as_str()
        )));
        let language_form = body
            .split("class=\"language-form\"")
            .nth(1)
            .unwrap()
            .split("</form>")
            .next()
            .unwrap();
        assert!(language_form.contains(&format!(
            "name=\"platform\" value=\"{}\"",
            platform.as_str()
        )));
        assert!(language_form.contains(">Apply</button>"));
        if platform == PlatformKind::Matrix {
            assert!(body.contains("encrypted rooms"));
        }
    }
    let before = app.drafts.lock()[&wizard_key(&admin.token)].platform_attempt;
    let response = router
        .clone()
        .oneshot(request(
            "GET",
            "/setup?step=5&platform=telegram",
            Some(&admin.token),
            "",
        ))
        .await
        .unwrap();
    let body = response_text(response).await;
    assert!(body.contains("platform-form"));
    assert!(!body.contains("complete-form"));
    let drafts = app.drafts.lock();
    let draft = &drafts[&wizard_key(&admin.token)];
    assert_eq!(draft.selected_platform, PlatformKind::Matrix);
    assert_eq!(
        draft.application.as_ref().unwrap().platform,
        PlatformKind::Matrix
    );
    assert!(draft.fit_completed);
    assert_eq!(draft.platform_attempt, before);
    assert_eq!(draft.values["DISCORD_TOKEN"], "private-discord-token");
}

#[tokio::test]
async fn slack_manifest_is_an_authenticated_exact_embedded_download() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let response = routes::router(app)
        .oneshot(request(
            "GET",
            "/setup/slack-app-manifest.yml",
            Some(&admin.token),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"slack-app-manifest.yml\""
    );
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/yaml; charset=utf-8"
    );
    assert_eq!(
        response_text(response).await,
        include_str!("../../docs/slack-app-manifest.yml")
    );
}

#[tokio::test]
async fn wizard_fit_and_final_use_only_selected_platform_fields() {
    use crate::platform::types::PlatformKind;
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    for (platform, access, value, owner_key) in [
        (PlatformKind::Discord, "CHANNELS", "123", "discord:user:42"),
        (
            PlatformKind::Telegram,
            "TELEGRAM_ROOMS",
            "-100123",
            "telegram:user:42",
        ),
        (
            PlatformKind::Slack,
            "SLACK_CHANNELS",
            "C12345678",
            "slack:user:T12345678:U12345678",
        ),
        (
            PlatformKind::Matrix,
            "MATRIX_ROOMS",
            "!room:example.org",
            "matrix:user:%40alice%3Aexample.org",
        ),
    ] {
        platform_draft(&app, &admin.token, platform);
        app.drafts
            .lock()
            .get_mut(&wizard_key(&admin.token))
            .unwrap()
            .fit_completed = false;
        let response = router
            .clone()
            .oneshot(request("GET", "/setup?step=4", Some(&admin.token), ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        for field in ["BOT_LANG", "TZ_OFFSET", "OWNER_KEY", "NEWS_RSS", access] {
            assert!(body.contains(&format!("name=\"{field}\"")));
        }
        if platform != PlatformKind::Discord {
            for field in [
                "DISCORD_TOKEN",
                "OWNER_ID",
                "GUILD_ID",
                "CHANNELS",
                "NEWS_CHANNEL",
            ] {
                assert!(!body.contains(&format!("name=\"{field}\"")));
            }
        }
        let form: HashMap<&str, &str> = [
            ("csrf", admin.info.csrf.as_str()),
            ("BOT_LANG", "en"),
            ("TZ_OFFSET", "5.5"),
            ("OWNER_KEY", owner_key),
            (access, value),
            ("NEWS_RSS", ""),
            ("DISCORD_TOKEN", "must-not-be-accepted"),
            (
                "GUILD_ID",
                if platform == PlatformKind::Discord {
                    "123"
                } else {
                    "not-a-discord-id"
                },
            ),
        ]
        .into_iter()
        .collect();
        let mut encoded = reqwest::Url::parse("http://localhost/").unwrap();
        encoded.query_pairs_mut().extend_pairs(form);
        let response = router
            .clone()
            .oneshot(request(
                "POST",
                "/ui/setup/fit",
                Some(&admin.token),
                encoded.query().unwrap(),
            ))
            .await
            .unwrap();
        if platform == PlatformKind::Matrix && !cfg!(feature = "matrix") {
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/html; charset=utf-8"
            );
            assert!(response_text(response)
                .await
                .contains("Some values need attention."));
            assert!(!app.drafts.lock()[&wizard_key(&admin.token)].fit_completed);
            assert!(!_directory.path().join(".env").exists());
            continue;
        }
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        {
            let drafts = app.drafts.lock();
            let draft = &drafts[&wizard_key(&admin.token)];
            assert!(draft.fit_completed);
            assert_eq!(draft.values["OWNER_KEY"], owner_key);
            assert_eq!(draft.values["DISCORD_TOKEN"], "private-discord-token");
        }
        let response = router
            .clone()
            .oneshot(request("GET", "/setup?step=5", Some(&admin.token), ""))
            .await
            .unwrap();
        let body = response_text(response).await;
        assert!(body.contains("name=\"access_confirmed\" value=\"on\""));
        assert!(!body.contains("name=\"intents\""));
        assert!(body.contains("not proof of live readiness"));
        assert!(body.contains("/settings?lang=en"));
        match platform {
            PlatformKind::Telegram => {
                assert!(body.contains("https://t.me/wizardbot?startgroup=true"));
                assert!(body.contains("did not report a privacy setting"));
                assert!(!body.contains("discord.com/oauth2"));
            }
            PlatformKind::Slack => {
                assert!(body.contains("/invite @your-bot"));
                assert!(!body.contains("discord.com/oauth2"));
            }
            PlatformKind::Matrix => {
                assert!(body.contains("Encrypted rooms are unsupported"));
                assert!(!body.contains("discord.com/oauth2"));
            }
            _ => assert!(body.contains("discord.com/oauth2")),
        }
    }
}

#[tokio::test]
async fn telegram_final_reports_verified_privacy_and_honest_unknown() {
    use crate::platform::types::PlatformKind;
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    for (privacy, message) in [
        (Some(true), "Privacy mode is ON"),
        (Some(false), "Privacy mode is OFF"),
        (None, "did not report a privacy setting"),
    ] {
        platform_draft(&app, &admin.token, PlatformKind::Telegram);
        app.drafts
            .lock()
            .get_mut(&wizard_key(&admin.token))
            .unwrap()
            .application
            .as_mut()
            .unwrap()
            .privacy_mode = privacy;
        let response = router
            .clone()
            .oneshot(request("GET", "/setup?step=5", Some(&admin.token), ""))
            .await
            .unwrap();
        assert!(response_text(response).await.contains(message));
    }
}

#[tokio::test]
async fn wizard_rejects_bypass_and_legacy_finish_acknowledgement() {
    use crate::platform::types::PlatformKind;
    let (app, directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    for endpoint in ["/ui/setup/fit", "/ui/setup/provider", "/ui/setup/complete"] {
        let response = router
            .clone()
            .oneshot(request(
                "POST",
                endpoint,
                Some(&admin.token),
                &format!("csrf={}&access_confirmed=on", admin.info.csrf),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    platform_draft(&app, &admin.token, PlatformKind::Telegram);
    for checkbox in ["", "&intents=on", "&access_confirmed=false"] {
        let response = router
            .clone()
            .oneshot(request(
                "POST",
                "/ui/setup/complete",
                Some(&admin.token),
                &format!("csrf={}{}", admin.info.csrf, checkbox),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    app.drafts
        .lock()
        .get_mut(&wizard_key(&admin.token))
        .unwrap()
        .selected_platform = PlatformKind::Slack;
    let response = router
        .oneshot(request(
            "POST",
            "/ui/setup/complete",
            Some(&admin.token),
            &format!("csrf={}&access_confirmed=on", admin.info.csrf),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!directory.path().join(".env").exists());
}

#[test]
fn platform_probe_publication_is_compare_and_swap_and_reuses_saved_keys() {
    use crate::platform::types::PlatformKind;
    let directory = tempfile::tempdir().unwrap();
    let config = config::Config::load(directory.path().join(".env"), &HashMap::new()).unwrap();
    let mut draft = setup::Draft::new(&config);
    draft
        .values
        .insert("DISCORD_TOKEN".into(), "unrelated-saved-secret".into());
    let first = routes::reserve_platform(&mut draft, PlatformKind::Telegram);
    let second = routes::reserve_platform(&mut draft, PlatformKind::Telegram);
    let credentials = HashMap::from([("TELEGRAM_TOKEN".into(), "verified-secret".into())]);
    assert!(routes::publish_platform(
        &mut draft,
        first,
        application(PlatformKind::Telegram),
        credentials.clone()
    )
    .is_err());
    assert!(draft.application.is_none());
    routes::publish_platform(
        &mut draft,
        second,
        application(PlatformKind::Telegram),
        credentials.clone(),
    )
    .unwrap();
    assert_eq!(draft.values["DISCORD_TOKEN"], "unrelated-saved-secret");
    assert_eq!(draft.values["TELEGRAM_TOKEN"], "verified-secret");
    draft.fit_completed = true;
    let late = routes::reserve_platform(&mut draft, PlatformKind::Telegram);
    routes::select_platform(&mut draft, PlatformKind::Slack);
    assert!(routes::publish_platform(
        &mut draft,
        late,
        application(PlatformKind::Telegram),
        credentials
    )
    .is_err());
    assert!(draft.application.is_none());
    assert!(!draft.fit_completed);
    let expired = routes::reserve_platform(&mut draft, PlatformKind::Slack);
    draft.expires = std::time::Instant::now();
    assert!(routes::publish_platform(
        &mut draft,
        expired,
        application(PlatformKind::Slack),
        HashMap::new()
    )
    .is_err());
    let mut old_draft = setup::Draft::new(&config);
    let pending = routes::reserve_platform(&mut old_draft, PlatformKind::Telegram);
    old_draft.expires = std::time::Instant::now();
    let mut recreated = setup::Draft::new(&config);
    let current = routes::reserve_platform(&mut recreated, PlatformKind::Telegram);
    assert_eq!(pending.1, current.1);
    assert_ne!(pending.0, current.0);
    assert!(routes::publish_platform(
        &mut recreated,
        pending,
        application(PlatformKind::Telegram),
        HashMap::from([("TELEGRAM_TOKEN".into(), "old-expired-secret".into())])
    )
    .is_err());
    assert!(recreated.application.is_none());
    assert!(!recreated
        .values
        .values()
        .any(|value| value == "old-expired-secret"));
    routes::publish_platform(
        &mut recreated,
        current,
        application(PlatformKind::Telegram),
        HashMap::from([("TELEGRAM_TOKEN".into(), "current-verified-secret".into())]),
    )
    .unwrap();
    assert_eq!(
        recreated.values["TELEGRAM_TOKEN"],
        "current-verified-secret"
    );
}

#[tokio::test]
async fn wizard_starts_selected_platform_without_discord_and_preserves_other_connections() {
    use crate::platform::types::PlatformKind;
    let mut platforms = vec![
        PlatformKind::Telegram,
        PlatformKind::Slack,
        PlatformKind::Discord,
    ];
    if cfg!(feature = "matrix") {
        platforms.push(PlatformKind::Matrix);
    }
    for platform in platforms {
        let (mut app, directory) = fixture().await;
        std::fs::write(
            directory.path().join(".env"),
            "TELEGRAM_TOKEN=preserved-telegram-secret\nTELEGRAM_ROOMS=telegram:room:-100123\n",
        )
        .unwrap();
        *app.config.lock() =
            config::Config::load(directory.path().join(".env"), &HashMap::new()).unwrap();
        let (start, mut receiver) = mpsc::channel(1);
        Arc::get_mut(&mut app).unwrap().start = start;
        let admin = owner(&app).await;
        platform_draft(&app, &admin.token, platform);
        {
            let mut drafts = app.drafts.lock();
            let draft = drafts.get_mut(&wizard_key(&admin.token)).unwrap();
            if platform != PlatformKind::Discord {
                draft.values.insert("DISCORD_TOKEN".into(), String::new());
            }
            if platform == PlatformKind::Telegram {
                draft
                    .values
                    .insert("TELEGRAM_TOKEN".into(), "preserved-telegram-secret".into());
                draft
                    .values
                    .insert("TELEGRAM_ROOMS".into(), "telegram:room:-100123".into());
            } else {
                // Unrelated imported draft values must never replace the saved connection.
                draft.values.insert(
                    "TELEGRAM_TOKEN".into(),
                    "must-not-overwrite-unrelated".into(),
                );
                draft
                    .values
                    .insert("TELEGRAM_ROOMS".into(), "telegram:room:-999".into());
            }
        }
        let response = routes::router(app.clone())
            .oneshot(request(
                "POST",
                "/ui/setup/complete",
                Some(&admin.token),
                &format!("csrf={}&access_confirmed=on", admin.info.csrf),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let config = receiver.recv().await.unwrap();
        if platform != PlatformKind::Discord {
            assert!(config.get("DISCORD_TOKEN").is_empty());
        }
        assert_eq!(config.get("TELEGRAM_TOKEN"), "preserved-telegram-secret");
        assert_eq!(config.get("TELEGRAM_ROOMS"), "telegram:room:-100123");
        assert!(!config
            .get(routes::platform_credentials(platform)[0])
            .is_empty());
        assert!(directory.path().join(".env").is_file());
    }
}

#[tokio::test]
async fn wizard_finish_keeps_environment_managed_credentials_out_of_file() {
    use crate::platform::types::PlatformKind;
    let (mut app, directory) = fixture().await;
    let config = config::Config::load(
        directory.path().join(".env"),
        &HashMap::from([
            ("TELEGRAM_TOKEN".into(), "environment-private-token".into()),
            ("OPENROUTER_KEY".into(), "environment-provider-key".into()),
        ]),
    )
    .unwrap();
    *app.config.lock() = config;
    let (start, mut receiver) = mpsc::channel(1);
    Arc::get_mut(&mut app).unwrap().start = start;
    let admin = owner(&app).await;
    let mut draft = setup::Draft::new(&app.config.lock());
    draft.selected_platform = PlatformKind::Telegram;
    draft.application = Some(application(PlatformKind::Telegram));
    draft.provider_verified = true;
    draft.fit_completed = true;
    app.drafts.lock().insert(wizard_key(&admin.token), draft);
    let response = routes::router(app.clone())
        .oneshot(request(
            "POST",
            "/ui/setup/complete",
            Some(&admin.token),
            &format!("csrf={}&access_confirmed=on", admin.info.csrf),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let config = receiver.recv().await.unwrap();
    assert_eq!(config.get("TELEGRAM_TOKEN"), "environment-private-token");
    assert_eq!(config.get("OPENROUTER_KEY"), "environment-provider-key");
    let file = std::fs::read_to_string(directory.path().join(".env")).unwrap();
    assert!(!file.contains("TELEGRAM_TOKEN="));
    assert!(!file.contains("OPENROUTER_KEY="));
    assert!(!file.contains("environment-private-token"));
    assert!(!file.contains("environment-provider-key"));
}

#[tokio::test]
async fn wizard_native_credential_posts_reject_invalid_secrets_without_echo_or_save() {
    use crate::platform::types::PlatformKind;
    for (platform, credential) in [
        (PlatformKind::Discord, "DISCORD_TOKEN"),
        (PlatformKind::Telegram, "TELEGRAM_TOKEN"),
        (PlatformKind::Slack, "SLACK_APP_TOKEN"),
        (PlatformKind::Matrix, "MATRIX_ACCESS_TOKEN"),
    ] {
        // Claim/protect consume three of the six per-IP attempts; isolate each probe case.
        let (app, directory) = fixture().await;
        let admin = owner(&app).await;
        let router = routes::router(app.clone());
        platform_draft(&app, &admin.token, platform);
        let original = app.drafts.lock()[&wizard_key(&admin.token)].values[credential].clone();
        let mut encoded = reqwest::Url::parse("http://localhost/").unwrap();
        encoded.query_pairs_mut().extend_pairs([
            ("csrf", admin.info.csrf.as_str()),
            ("platform", platform.as_str()),
            (credential, "do-not-echo-secret\ninjected"),
        ]);
        let response = router
            .clone()
            .oneshot(request(
                "POST",
                "/ui/setup/platform",
                Some(&admin.token),
                encoded.query().unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = response_text(response).await;
        assert!(!body.contains("do-not-echo-secret"));
        let drafts = app.drafts.lock();
        let draft = &drafts[&wizard_key(&admin.token)];
        assert!(draft.application.is_none());
        assert!(!draft.fit_completed);
        assert_eq!(draft.values[credential], original);
        assert!(!directory.path().join(".env").exists());
    }
}

#[tokio::test]
async fn wizard_provider_switch_keeps_generic_key_editable_without_javascript() {
    use crate::platform::types::PlatformKind;
    for (provider, key_name, fixed_provider) in [
        ("openrouter", "OPENROUTER_KEY", false),
        ("mistral", "MISTRAL_KEY", false),
        ("openrouter", "OPENROUTER_KEY", true),
        ("mistral", "MISTRAL_KEY", true),
    ] {
        let (app, directory) = fixture().await;
        let mut environment = HashMap::from([(key_name.into(), "environment-private-key".into())]);
        if fixed_provider {
            environment.insert("PROVIDER".into(), provider.into());
        }
        *app.config.lock() =
            config::Config::load(directory.path().join(".env"), &environment).unwrap();
        let admin = owner(&app).await;
        platform_draft(&app, &admin.token, PlatformKind::Telegram);
        app.drafts
            .lock()
            .get_mut(&wizard_key(&admin.token))
            .unwrap()
            .values
            .insert("PROVIDER".into(), provider.into());
        let router = routes::router(app.clone());
        let response = router
            .clone()
            .oneshot(request("GET", "/setup?step=3", Some(&admin.token), ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        let key_input = body
            .split("id=\"provider-form-key\"")
            .nth(1)
            .unwrap()
            .split('>')
            .next()
            .unwrap();
        assert_eq!(key_input.contains(" readonly"), fixed_provider);
        let provider_select = body
            .split("id=\"provider-form-PROVIDER\"")
            .nth(1)
            .unwrap()
            .split('>')
            .next()
            .unwrap();
        assert_eq!(provider_select.contains(" disabled"), fixed_provider);
        assert!(!body.contains("environment-private-key"));
        // The submitted provider's environment-owned key still cannot be replaced.
        let response = router
            .oneshot(request(
                "POST",
                "/ui/setup/provider",
                Some(&admin.token),
                &format!(
                    "csrf={}&PROVIDER={}&key=replacement-not-allowed",
                    admin.info.csrf, provider
                ),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!directory.path().join(".env").exists());
    }
}

#[tokio::test]
async fn matrix_wizard_requires_token_and_never_promises_password_verification() {
    use crate::platform::types::PlatformKind;
    assert!(!routes::platform_credentials(PlatformKind::Matrix).contains(&"MATRIX_PASSWORD"));
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    platform_draft(&app, &admin.token, PlatformKind::Matrix);
    let router = routes::router(app);
    for lang in ["en", "tr"] {
        let response = router
            .clone()
            .oneshot(request(
                "GET",
                &format!("/setup?step=2&platform=matrix&lang={lang}"),
                Some(&admin.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_text(response).await;
        assert!(body.contains("name=\"MATRIX_ACCESS_TOKEN\""));
        assert!(!body.contains("name=\"MATRIX_PASSWORD\""));
        assert!(!body.contains("access token or account password"));
        assert!(!body.contains("instead supply the account password"));
        assert!(!body.contains("erişim tokeni veya hesap parolasını"));
        assert!(!body.contains("aşağıya hesap parolasını"));
    }
}

#[cfg(feature = "matrix")]
#[tokio::test]
async fn matrix_wizard_finish_preserves_existing_password_without_accepting_a_replacement() {
    use crate::platform::types::PlatformKind;
    let (mut app, directory) = fixture().await;
    std::fs::write(directory.path().join(".env"),
        "MATRIX_HOMESERVER=https://matrix.example.org\nMATRIX_USER=@wizardbot:example.org\nMATRIX_ACCESS_TOKEN=saved-matrix-token\nMATRIX_PASSWORD=preserved-matrix-password\n").unwrap();
    *app.config.lock() =
        config::Config::load(directory.path().join(".env"), &HashMap::new()).unwrap();
    let (start, mut receiver) = mpsc::channel(1);
    Arc::get_mut(&mut app).unwrap().start = start;
    let admin = owner(&app).await;
    platform_draft(&app, &admin.token, PlatformKind::Matrix);
    app.drafts
        .lock()
        .get_mut(&wizard_key(&admin.token))
        .unwrap()
        .values
        .insert(
            "MATRIX_PASSWORD".into(),
            "draft-password-must-not-be-saved".into(),
        );
    let response = routes::router(app)
        .oneshot(request(
            "POST",
            "/ui/setup/complete",
            Some(&admin.token),
            &format!(
                "csrf={}&access_confirmed=on&MATRIX_PASSWORD=form-password-must-not-be-saved",
                admin.info.csrf
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let config = receiver.recv().await.unwrap();
    assert_eq!(config.get("MATRIX_PASSWORD"), "preserved-matrix-password");
    let file = std::fs::read_to_string(directory.path().join(".env")).unwrap();
    assert!(file.contains("preserved-matrix-password"));
    assert!(!file.contains("draft-password-must-not-be-saved"));
    assert!(!file.contains("form-password-must-not-be-saved"));
}

#[tokio::test]
async fn wizard_blank_telegram_and_slack_allowlists_are_default_deny_and_failed_fit_invalidates() {
    use crate::platform::types::PlatformKind;
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    for (platform, access) in [
        (PlatformKind::Telegram, "TELEGRAM_ROOMS"),
        (PlatformKind::Slack, "SLACK_CHANNELS"),
    ] {
        platform_draft(&app, &admin.token, platform);
        let response = router
            .clone()
            .oneshot(request(
                "POST",
                "/ui/setup/fit",
                Some(&admin.token),
                &format!("csrf={}&{}=", admin.info.csrf, access),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            app.drafts.lock()[&wizard_key(&admin.token)].values[access],
            ""
        );
        let response = router
            .clone()
            .oneshot(request(
                "POST",
                "/ui/setup/fit",
                Some(&admin.token),
                &format!("csrf={}&{}=invalid-room-id", admin.info.csrf, access),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!app.drafts.lock()[&wizard_key(&admin.token)].fit_completed);
        let response = router
            .clone()
            .oneshot(request("GET", "/setup?step=5", Some(&admin.token), ""))
            .await
            .unwrap();
        assert!(!response_text(response).await.contains("complete-form"));
    }
}

#[tokio::test]
async fn dashboard_refresh_keeps_localized_empty_states_and_write_only_secrets() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app);
    let response = router
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/status?lang=tr",
            Some(&admin.token),
            "",
        ))
        .await
        .unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(
        snapshot["display"]["lifecycle"],
        views::text(views::Language::Tr, "panel.state.unconfigured").unwrap()
    );
    assert_eq!(
        snapshot["display"]["name"],
        views::text(views::Language::Tr, "panel.state.no_name").unwrap()
    );
    assert_eq!(
        snapshot["display"]["days"],
        views::text(views::Language::Tr, "panel.state.not_reported").unwrap()
    );
    assert!(snapshot.get("global_mood").is_none());
    assert!(snapshot.get("key").is_none());
    assert_eq!(snapshot["connection"], "offline");
    assert_eq!(snapshot["connections"], serde_json::json!([]));
    assert_eq!(snapshot["spaces"], 0);
    assert_eq!(snapshot["rooms"], 0);
    assert_eq!(
        snapshot["display"]["connections"],
        views::text(views::Language::Tr, "panel.overview.no_connections").unwrap()
    );
    let response = router
        .oneshot(request("GET", "/?lang=tr", Some(&admin.token), ""))
        .await
        .unwrap();
    let body = String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(!body.contains("/api/v1/status?lang=tr"));
    assert!(!body.contains("data-status-key=\"display.lifecycle\""));
    assert!(body.contains(views::text(views::Language::Tr, "panel.overview.finish_setup").unwrap()));
    assert!(body.contains("href=\"/setup?lang=tr\""));
    assert!(!body.contains(views::text(views::Language::Tr, "panel.state.not_reported").unwrap()));
    assert!(body.contains(views::PROJECT_NAME));
    for href in ["/memory", "/personality", "/actions", "/usage"] {
        assert!(body.contains(&format!("href=\"{href}?lang=tr\"")));
    }
}

#[tokio::test]
async fn dashboard_lifecycle_never_claims_transport_readiness_without_snapshots() {
    let (app, _directory) = fixture().await;
    for (lifecycle, value) in [
        (Lifecycle::Unconfigured, "unconfigured"),
        (Lifecycle::ConfiguredStarting, "configured_starting"),
        (Lifecycle::Running, "running"),
        (Lifecycle::ConfigurationError, "configuration_error"),
        (Lifecycle::Stopping, "stopping"),
    ] {
        app.runtime.lock().lifecycle = lifecycle;
        for language in [views::Language::En, views::Language::Tr] {
            let snapshot = dashboard::snapshot(&app, language).await;
            assert_eq!(snapshot["lifecycle"], value);
            assert_eq!(snapshot["connection"], "offline");
            assert_eq!(snapshot["spaces"], 0);
            assert_eq!(snapshot["rooms"], 0);
            assert_eq!(snapshot["connections"], serde_json::json!([]));
            assert_eq!(
                snapshot["display"]["connection"],
                views::text(language, "panel.state.offline").unwrap()
            );
            assert_eq!(
                snapshot["display"]["connections"],
                views::text(language, "panel.overview.no_connections").unwrap()
            );
            assert!(snapshot.get("usage").is_none());
            assert!(snapshot.get("cost").is_none());
        }
    }
}

#[tokio::test]
async fn pre_cancelled_runtime_owner_does_not_activate_queued_configuration() {
    let (app, directory) = fixture().await;
    let startup = HashMap::from([
        (
            "DISCORD_TOKEN".into(),
            "cancelled-runtime-discord-token".into(),
        ),
        ("PROVIDER".into(), "openrouter".into()),
        (
            "OPENROUTER_KEY".into(),
            "cancelled-runtime-provider-key".into(),
        ),
    ]);
    let candidate = config::Config::load(directory.path().join(".env"), &startup).unwrap();
    candidate.validate_runtime().unwrap();
    let original_runtime_token = config::runtime_value("DISCORD_TOKEN");
    let (start, receiver) = mpsc::channel(1);
    start.send(candidate).await.unwrap();
    let (_stop_sender, stop) = watch::channel(true);
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        runtime_owner(app.clone(), receiver, stop),
    )
    .await
    .unwrap()
    .unwrap();
    let runtime = app.runtime.lock();
    assert_eq!(runtime.lifecycle, Lifecycle::Unconfigured);
    assert!(!runtime.attempted);
    assert!(runtime.bot.is_none());
    assert_eq!(
        config::runtime_value("DISCORD_TOKEN"),
        original_runtime_token
    );
    assert!(!directory.path().join(".env").exists());
}

#[test]
fn both_panel_languages_have_the_same_complete_key_set() {
    use std::collections::BTreeSet;
    let english: HashMap<String, String> =
        serde_json::from_str(include_str!("../../langs/en.json")).unwrap();
    let turkish: HashMap<String, String> =
        serde_json::from_str(include_str!("../../langs/tr.json")).unwrap();
    let keys = |table: &HashMap<String, String>| {
        table
            .keys()
            .filter(|key| key.starts_with("panel."))
            .cloned()
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(keys(&english), keys(&turkish));
    assert!(english
        .iter()
        .chain(turkish.iter())
        .filter(|(key, _)| key.starts_with("panel."))
        .all(|(_, value)| !value.is_empty()));
}

fn settings_fixture(app: &App) {
    let path = app.config.lock().path().to_owned();
    std::fs::write(&path,"# keep this comment\nDISCORD_TOKEN=settings-private-discord-credential\nTELEGRAM_TOKEN=settings-private-telegram-credential\nTELEGRAM_ROOMS=-100123\nSLACK_APP_TOKEN=settings-private-slack-app-credential\nSLACK_BOT_TOKEN=settings-private-slack-bot-credential\nSLACK_CHANNELS=C123,G456\nOPENROUTER_KEY=settings-private-provider-credential\nBOT_LANG=en\nSTEALTH=false\nCUSTOM_SETTING='untouched # value'\n").unwrap();
    *app.config.lock() = config::Config::load(path, &HashMap::new()).unwrap();
}

#[tokio::test]
async fn settings_forms_preserve_unposted_groups_and_never_echo_credentials() {
    let (app, directory) = fixture().await;
    settings_fixture(&app);
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    let response = router
        .clone()
        .oneshot(request("GET", "/settings", Some(&admin.token), ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(response.into_body(), 256 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(!html.contains("settings-private"));
    assert!(html.contains("value=\"off\" selected"));
    assert!(html.contains("/ui/settings/test-platform"));
    assert!(html.contains("Other configuration entries"));
    assert!(html.contains("name=\"DISCORD_TOKEN\" type=\"password\""));
    assert!(html.contains("name=\"TELEGRAM_TOKEN\" type=\"password\""));
    assert!(html.contains("name=\"clear_TELEGRAM_TOKEN\""));
    assert!(html.contains("name=\"TELEGRAM_ROOMS\" type=\"text\" value=\"telegram:room:-100123\""));
    assert!(html.contains("Approved Telegram rooms"));
    assert!(html.contains("Blank denies all Telegram rooms"));
    for key in ["SLACK_APP_TOKEN", "SLACK_BOT_TOKEN"] {
        assert!(html.contains(&format!("name=\"{key}\" type=\"password\"")));
        assert!(html.contains(&format!("name=\"clear_{key}\"")));
    }
    assert!(html.contains("name=\"SLACK_CHANNELS\" type=\"text\" value=\"C123,G456\""));
    assert!(html.contains("name=\"SLACK_HISTORY_TIER\""));
    assert!(html.contains("value=\"internal\" selected"));
    assert!(html.contains("value=\"restricted\""));
    assert!(html.contains("name=\"SLACK_TEST_CHANNEL\" type=\"text\" value=\"\""));
    assert!(html.contains("Approved Slack channels"));
    assert!(html.contains("Blank denies all Slack channels"));
    for key in ["MATRIX_ACCESS_TOKEN", "MATRIX_PASSWORD"] {
        assert!(html.contains(&format!("name=\"{key}\" type=\"password\"")));
        assert!(html.contains(&format!("name=\"clear_{key}\"")));
    }
    for key in ["MATRIX_HOMESERVER", "MATRIX_USER", "MATRIX_ROOMS"] {
        assert!(html.contains(&format!("name=\"{key}\" type=\"text\"")));
    }
    assert!(html.contains("Approved Matrix rooms"));
    assert!(html.contains("Blank approves only owner invitations"));
    let revision = app.config.lock().revision().to_owned();
    let body = format!("csrf={}&revision={revision}&BOT_LANG=tr", admin.info.csrf);
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            "/ui/settings?lang=tr",
            Some(&admin.token),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        "/settings?saved=1&lang=tr"
    );
    let saved = std::fs::read_to_string(directory.path().join(".env")).unwrap();
    assert!(saved.contains("# keep this comment"));
    assert!(saved.contains("CUSTOM_SETTING='untouched # value'"));
    assert!(saved.contains("DISCORD_TOKEN=settings-private-discord-credential"));
    assert!(saved.contains("OPENROUTER_KEY=settings-private-provider-credential"));
    assert!(saved.contains("TELEGRAM_TOKEN=settings-private-telegram-credential"));
    assert!(saved.contains("TELEGRAM_ROOMS=-100123"));
    assert!(saved.contains("SLACK_APP_TOKEN=settings-private-slack-app-credential"));
    assert!(saved.contains("SLACK_BOT_TOKEN=settings-private-slack-bot-credential"));
    assert!(saved.contains("SLACK_CHANNELS=C123,G456"));
    assert_eq!(app.config.lock().get("BOT_LANG"), "en");
    assert!(
        app.config
            .lock()
            .fields()
            .iter()
            .find(|view| view.spec.key == "BOT_LANG")
            .unwrap()
            .pending_restart
    );
    let response = router
        .oneshot(request("GET", "/api/v1/settings", Some(&admin.token), ""))
        .await
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 256 * 1024).await.unwrap()).unwrap();
    assert!(!value.to_string().contains("settings-private"));
    let secret = value["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["spec"]["key"] == "OPENROUTER_KEY")
        .unwrap();
    assert!(secret["saved"]["configured"].as_bool().unwrap());
    assert!(secret["saved"]["value"].is_null());
    let telegram = value["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["spec"]["key"] == "TELEGRAM_TOKEN")
        .unwrap();
    assert_eq!(telegram["spec"]["kind"], "secret");
    assert_eq!(telegram["spec"]["application"], "restart");
    assert_eq!(telegram["effective"]["configured"], true);
    assert_eq!(telegram["saved"]["configured"], true);
    assert!(telegram["effective"]["value"].is_null());
    assert!(telegram["saved"]["value"].is_null());
    assert!(telegram["default"]["value"].is_null());
    let rooms = value["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["spec"]["key"] == "TELEGRAM_ROOMS")
        .unwrap();
    assert_eq!(rooms["spec"]["kind"], "telegram_rooms");
    assert_eq!(rooms["effective"]["value"], "telegram:room:-100123");
    for key in ["SLACK_APP_TOKEN", "SLACK_BOT_TOKEN"] {
        let slack = value["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|view| view["spec"]["key"] == key)
            .unwrap();
        assert_eq!(slack["spec"]["kind"], "secret");
        assert_eq!(slack["spec"]["application"], "restart");
        assert_eq!(slack["effective"]["configured"], true);
        assert_eq!(slack["saved"]["configured"], true);
        assert!(slack["effective"]["value"].is_null());
        assert!(slack["saved"]["value"].is_null());
        assert!(slack["default"]["value"].is_null());
    }
    let channels = value["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["spec"]["key"] == "SLACK_CHANNELS")
        .unwrap();
    assert_eq!(channels["spec"]["kind"], "slack_channels");
    assert_eq!(channels["effective"]["value"], "C123,G456");
}

#[tokio::test]
async fn settings_validation_returns_localized_field_errors_with_matching_request_id() {
    let (app, directory) = fixture().await;
    settings_fixture(&app);
    let admin = owner(&app).await;
    let original = std::fs::read(directory.path().join(".env")).unwrap();
    let body=serde_json::json!({"csrf":admin.info.csrf,"revision":app.config.lock().revision(),"patch":{"TZ_OFFSET":{"operation":"replace","value":"99"}}}).to_string();
    let mut request = request(
        "POST",
        "/api/v1/settings/validate?lang=tr",
        Some(&admin.token),
        &body,
    );
    request
        .headers_mut()
        .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    let response = routes::router(app.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let identifier = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(value["request_id"], identifier);
    assert!(value["field_errors"]["TZ_OFFSET"]
        .as_str()
        .is_some_and(|value| !value.is_empty() && !value.starts_with("panel.")));
    assert_eq!(
        std::fs::read(directory.path().join(".env")).unwrap(),
        original
    );
}

#[tokio::test]
async fn partial_slack_pair_and_native_channel_errors_are_localized_without_saving() {
    let (app, directory) = fixture().await;
    settings_fixture(&app);
    let admin = owner(&app).await;
    let original = std::fs::read(directory.path().join(".env")).unwrap();
    for language in [views::Language::En, views::Language::Tr] {
        for (key, operation, code) in [
            (
                "SLACK_APP_TOKEN",
                serde_json::json!({"operation":"clear"}),
                "panel.config.slack_token_pair",
            ),
            (
                "SLACK_BOT_TOKEN",
                serde_json::json!({"operation":"clear"}),
                "panel.config.slack_token_pair",
            ),
            (
                "SLACK_CHANNELS",
                serde_json::json!({"operation":"replace","value":"#private-invalid-channel"}),
                "panel.config.invalid_slack_channel",
            ),
            (
                "SLACK_TEST_CHANNEL",
                serde_json::json!({"operation":"replace","value":"D123"}),
                "panel.config.invalid_slack_channel",
            ),
            (
                "SLACK_HISTORY_TIER",
                serde_json::json!({"operation":"replace","value":"unsupported"}),
                "panel.config.invalid_choice",
            ),
        ] {
            let mut patch = serde_json::Map::new();
            patch.insert(key.into(), operation);
            let body = serde_json::json!({
                "csrf":admin.info.csrf,
                "revision":app.config.lock().revision(),
                "patch":patch,
            })
            .to_string();
            let suffix = if matches!(language, views::Language::Tr) {
                "tr"
            } else {
                "en"
            };
            let mut request = request(
                "POST",
                &format!("/api/v1/settings/validate?lang={suffix}"),
                Some(&admin.token),
                &body,
            );
            request
                .headers_mut()
                .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
            let response = routes::router(app.clone()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            let identifier = response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned();
            let value: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                    .unwrap();
            assert_eq!(value["request_id"], identifier);
            assert_eq!(
                value["field_errors"][key],
                views::text(language, code).unwrap()
            );
            assert!(!value.to_string().contains("settings-private"));
            assert!(!value.to_string().contains("#private-invalid-channel"));
            assert_eq!(
                std::fs::read(directory.path().join(".env")).unwrap(),
                original
            );
        }
    }
}

#[tokio::test]
async fn telegram_room_validation_is_localized_and_does_not_save_invalid_approval() {
    let (app, directory) = fixture().await;
    settings_fixture(&app);
    let admin = owner(&app).await;
    let original = std::fs::read(directory.path().join(".env")).unwrap();
    let router = routes::router(app.clone());
    for language in ["en", "tr"] {
        let body = serde_json::json!({
            "csrf": admin.info.csrf,
            "revision": app.config.lock().revision(),
            "patch": {"TELEGRAM_ROOMS": {"operation": "replace", "value": "discord:room:42"}}
        })
        .to_string();
        let mut request = request(
            "POST",
            &format!("/api/v1/settings/validate?lang={language}"),
            Some(&admin.token),
            &body,
        );
        request
            .headers_mut()
            .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let identifier = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(value["request_id"], identifier);
        assert!(value["field_errors"]["TELEGRAM_ROOMS"]
            .as_str()
            .is_some_and(|message| !message.is_empty() && !message.starts_with("panel.")));
        assert!(!value.to_string().contains("settings-private"));
        assert_eq!(
            std::fs::read(directory.path().join(".env")).unwrap(),
            original
        );
    }
}

#[tokio::test]
async fn settings_tests_reject_invalid_credentials_without_saving_or_mutating_wizard() {
    let (app, directory) = fixture().await;
    settings_fixture(&app);
    let admin = owner(&app).await;
    let original = std::fs::read(directory.path().join(".env")).unwrap();
    let body = format!(
        "csrf={}&PROVIDER=invalid&key=supplied-secret",
        admin.info.csrf
    );
    let response = routes::router(app.clone())
        .oneshot(request(
            "POST",
            "/ui/settings/test-provider",
            Some(&admin.token),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = String::from_utf8(
        to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(!html.contains("supplied-secret"));
    assert!(html.contains("/settings?lang=en"));
    assert!(app.drafts.lock().is_empty());
    assert_eq!(
        std::fs::read(directory.path().join(".env")).unwrap(),
        original
    );
}

fn settings_probe_config(
    environment: &HashMap<String, String>,
) -> (tempfile::TempDir, config::Config) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(".env");
    std::fs::write(
        &path,
        "DISCORD_TOKEN=probe-private-discord\nTELEGRAM_TOKEN=probe-private-telegram\nSLACK_APP_TOKEN=xapp-probe-private-app\nSLACK_BOT_TOKEN=xoxb-probe-private-bot\nMATRIX_HOMESERVER=https://matrix.example.org\nMATRIX_USER=@bot:matrix.example.org\nMATRIX_ACCESS_TOKEN=probe-private-matrix\nMATRIX_PASSWORD=probe-private-password\nOPENROUTER_KEY=probe-private-provider\n",
    )
    .unwrap();
    let config = config::Config::load(path, environment).unwrap();
    (directory, config)
}

#[tokio::test]
async fn settings_has_four_collapsed_explicit_write_only_platform_tests_before_provider() {
    use crate::platform::types::PlatformKind;
    let (app, _directory) = fixture().await;
    let (_probe_directory, config) = settings_probe_config(&HashMap::new());
    *app.config.lock() = config;
    let admin = owner(&app).await;
    let router = routes::router(app);
    for language in ["en", "tr"] {
        let response = router
            .clone()
            .oneshot(request(
                "GET",
                &format!("/settings?lang={language}"),
                Some(&admin.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let html = response_text(response).await;
        assert!(!html.contains("probe-private"));
        assert_eq!(
            html.matches("action=\"/ui/settings/test-platform?").count(),
            4
        );
        let provider = html.find("id=\"settings-provider-test\"").unwrap();
        for platform in [
            PlatformKind::Discord,
            PlatformKind::Telegram,
            PlatformKind::Slack,
            PlatformKind::Matrix,
        ] {
            let name = platform.as_str();
            let card_start = html
                .find(&format!(
                    "<details class=\"card\" id=\"settings-{name}-test-card\">"
                ))
                .unwrap();
            assert!(card_start < provider);
            let card = html[card_start..].split("</details>").next().unwrap();
            assert!(!card.split('>').next().unwrap().contains("open"));
            assert!(card.contains(&format!("id=\"settings-{name}-test\"")));
            assert!(card.contains(&format!("name=\"platform\" value=\"{name}\"")));
            assert!(card.contains(&format!("name=\"csrf\" value=\"{}\"", admin.info.csrf)));
            assert_eq!(card.matches("<button ").count(), 1);
            assert!(card.contains("<button class=\"button\" type=\"submit\""));
            assert!(card.contains("class=\"section-description\""));
            for &key in routes::platform_credentials(platform) {
                assert!(card.contains(&format!("name=\"{key}\"")));
                let help = card
                    .split(&format!("id=\"settings-{name}-test-{key}-help\">"))
                    .nth(1)
                    .unwrap()
                    .split("</p>")
                    .next()
                    .unwrap();
                assert!(!help.trim().is_empty());
                let guide = views::text(
                    views::Language::parse(language),
                    &format!("panel.setup.field.{key}.help"),
                )
                .unwrap();
                assert!(help.contains(guide.split('.').next().unwrap()));
                assert!(!help.contains("Requires restart"));
                assert!(!help.contains("Yeniden başlatma gerekir"));
                if key.contains("TOKEN") {
                    let input = card
                        .split(&format!("name=\"{key}\""))
                        .nth(1)
                        .unwrap()
                        .split('>')
                        .next()
                        .unwrap();
                    assert!(input.contains("type=\"password\""));
                    assert!(!input.contains(" value=") || input.contains("value=\"\""));
                }
            }
            for other in [
                "DISCORD_TOKEN",
                "TELEGRAM_TOKEN",
                "SLACK_APP_TOKEN",
                "SLACK_BOT_TOKEN",
                "MATRIX_HOMESERVER",
                "MATRIX_USER",
                "MATRIX_ACCESS_TOKEN",
                "MATRIX_PASSWORD",
                "OPENROUTER_KEY",
            ] {
                if !routes::platform_credentials(platform).contains(&other) {
                    assert!(!card.contains(&format!("name=\"{other}\"")));
                }
            }
        }
        assert!(html.contains("action=\"/ui/settings/test-provider?"));
    }
}

#[test]
fn settings_probe_values_select_only_required_platform_keys_and_blank_uses_saved() {
    use crate::platform::types::PlatformKind;
    let (_directory, config) = settings_probe_config(&HashMap::new());
    for platform in [
        PlatformKind::Discord,
        PlatformKind::Telegram,
        PlatformKind::Slack,
        PlatformKind::Matrix,
    ] {
        if platform == PlatformKind::Matrix && !cfg!(feature = "matrix") {
            continue;
        }
        let keys = routes::platform_credentials(platform);
        let mut input = HashMap::from([
            ("csrf".into(), "irrelevant-csrf".into()),
            ("platform".into(), platform.as_str().into()),
            (
                "OPENROUTER_KEY".into(),
                "do-not-forward-provider-secret".into(),
            ),
            ("MATRIX_PASSWORD".into(), "do-not-forward-password".into()),
            ("TELEGRAM_ROOMS".into(), "not-a-room".into()),
        ]);
        for &key in keys {
            input.insert(key.into(), String::new());
        }
        let saved = settings::platform_probe_values(&config, platform, &input).unwrap();
        assert_eq!(saved.len(), keys.len());
        for &key in keys {
            assert_eq!(saved[key], config.get(key));
        }
        for other in [
            PlatformKind::Discord,
            PlatformKind::Telegram,
            PlatformKind::Slack,
            PlatformKind::Matrix,
        ] {
            if other != platform {
                for &key in routes::platform_credentials(other) {
                    input.insert(key.into(), "cross-platform-secret\ninvalid".into());
                }
            }
        }
        let credential = keys.iter().find(|key| key.contains("TOKEN")).unwrap();
        input.insert((*credential).into(), "  supplied-private-token  ".into());
        let supplied = settings::platform_probe_values(&config, platform, &input).unwrap();
        assert_eq!(supplied.len(), keys.len());
        assert_eq!(supplied[*credential], "supplied-private-token");
        for &key in keys {
            if key != *credential {
                assert_eq!(supplied[key], config.get(key));
            }
        }
        input.insert((*credential).into(), "invalid-secret\ninjected".into());
        assert!(settings::platform_probe_values(&config, platform, &input).is_err());
    }
}

#[test]
fn settings_probe_values_require_each_selected_credential_not_other_platforms() {
    use crate::platform::types::PlatformKind;
    let directory = tempfile::tempdir().unwrap();
    let config = config::Config::load(directory.path().join(".env"), &HashMap::new()).unwrap();
    for platform in [
        PlatformKind::Discord,
        PlatformKind::Telegram,
        PlatformKind::Slack,
        PlatformKind::Matrix,
    ] {
        if platform == PlatformKind::Matrix && !cfg!(feature = "matrix") {
            continue;
        }
        let keys = routes::platform_credentials(platform);
        let complete: HashMap<String, String> = keys
            .iter()
            .map(|&key| {
                let value = match key {
                    "MATRIX_HOMESERVER" => "  https://matrix.example.org  ",
                    "MATRIX_USER" => "  @bot:matrix.example.org  ",
                    _ => "  selected-private-token  ",
                };
                (key.into(), value.into())
            })
            .collect();
        let values = settings::platform_probe_values(&config, platform, &complete).unwrap();
        for &key in keys {
            assert_eq!(values[key], complete[key].trim());
            let mut missing = complete.clone();
            missing.remove(key);
            assert!(settings::platform_probe_values(&config, platform, &missing).is_err());
        }
    }
}

#[test]
fn settings_probe_values_never_override_environment_managed_secrets() {
    use crate::platform::types::PlatformKind;
    for (platform, key) in [
        (PlatformKind::Discord, "DISCORD_TOKEN"),
        (PlatformKind::Telegram, "TELEGRAM_TOKEN"),
        (PlatformKind::Slack, "SLACK_APP_TOKEN"),
        (PlatformKind::Slack, "SLACK_BOT_TOKEN"),
        (PlatformKind::Matrix, "MATRIX_ACCESS_TOKEN"),
    ] {
        if platform == PlatformKind::Matrix && !cfg!(feature = "matrix") {
            continue;
        }
        let env = HashMap::from([(key.into(), "environment-private-token".into())]);
        let (_directory, config) = settings_probe_config(&env);
        let blank = HashMap::from([(key.into(), String::new())]);
        let values = settings::platform_probe_values(&config, platform, &blank).unwrap();
        assert_eq!(values[key], "environment-private-token");
        for replacement in ["replacement-private-token", "environment-private-token"] {
            let input = HashMap::from([(key.into(), replacement.into())]);
            assert!(settings::platform_probe_values(&config, platform, &input).is_err());
        }
        let env = HashMap::from([(key.into(), String::new())]);
        let (_directory, config) = settings_probe_config(&env);
        assert!(settings::platform_probe_values(&config, platform, &blank).is_err());
        let input = HashMap::from([(key.into(), "replacement-private-token".into())]);
        assert!(settings::platform_probe_values(&config, platform, &input).is_err());
    }
}

#[cfg(feature = "matrix")]
#[test]
fn settings_matrix_probe_is_token_only_typed_and_preserves_saved_password() {
    use crate::platform::types::PlatformKind;
    let (directory, config) = settings_probe_config(&HashMap::new());
    let original = std::fs::read(config.path()).unwrap();
    let input = HashMap::from([
        (
            "MATRIX_PASSWORD".into(),
            "replacement-password\nignored".into(),
        ),
        (
            "MATRIX_ACCESS_TOKEN".into(),
            "  supplied-private-matrix-token  ".into(),
        ),
    ]);
    let values = settings::platform_probe_values(&config, PlatformKind::Matrix, &input).unwrap();
    assert_eq!(values.len(), 3);
    assert!(!values.contains_key("MATRIX_PASSWORD"));
    assert_eq!(
        values["MATRIX_ACCESS_TOKEN"],
        "supplied-private-matrix-token"
    );
    assert_eq!(config.get("MATRIX_PASSWORD"), "probe-private-password");
    assert_eq!(std::fs::read(config.path()).unwrap(), original);
    for (key, value) in [
        (
            "MATRIX_HOMESERVER",
            "https://user:password@matrix.example.org",
        ),
        (
            "MATRIX_HOMESERVER",
            "https://matrix.example.org?token=private",
        ),
        ("MATRIX_USER", "bot-without-qualified-user-id"),
    ] {
        let invalid = HashMap::from([(key.into(), value.into())]);
        assert!(settings::platform_probe_values(&config, PlatformKind::Matrix, &invalid).is_err());
    }
    std::fs::write(
        directory.path().join(".env"),
        "MATRIX_HOMESERVER=https://matrix.example.org\nMATRIX_USER=@bot:matrix.example.org\nMATRIX_PASSWORD=probe-private-password\n",
    )
    .unwrap();
    let password_only = config.reload().unwrap();
    assert!(
        settings::platform_probe_values(&password_only, PlatformKind::Matrix, &HashMap::new())
            .is_err()
    );
}

#[cfg(feature = "matrix")]
#[test]
fn settings_matrix_probe_accepts_only_blank_or_exact_environment_nonsecret_values() {
    use crate::platform::types::PlatformKind;
    for (key, effective, replacement) in [
        (
            "MATRIX_HOMESERVER",
            "https://environment.example.org",
            "https://replacement.example.org",
        ),
        (
            "MATRIX_USER",
            "@environment:example.org",
            "@replacement:example.org",
        ),
    ] {
        let env = HashMap::from([(key.into(), effective.into())]);
        let (_directory, config) = settings_probe_config(&env);
        for supplied in ["", effective] {
            let input = HashMap::from([(key.into(), supplied.into())]);
            let values =
                settings::platform_probe_values(&config, PlatformKind::Matrix, &input).unwrap();
            assert_eq!(values[key], effective);
        }
        let input = HashMap::from([(key.into(), replacement.into())]);
        assert!(settings::platform_probe_values(&config, PlatformKind::Matrix, &input).is_err());
    }
}

#[cfg(not(feature = "matrix"))]
#[test]
fn settings_matrix_probe_rejects_disabled_feature_even_with_complete_credentials() {
    let (_directory, config) = settings_probe_config(&HashMap::new());
    assert!(settings::platform_probe_values(
        &config,
        crate::platform::types::PlatformKind::Matrix,
        &HashMap::new(),
    )
    .is_err());
}

#[tokio::test]
async fn settings_platform_tests_reject_supplied_and_saved_invalid_credentials_without_mutation() {
    use crate::platform::types::PlatformKind;
    for platform in [
        PlatformKind::Discord,
        PlatformKind::Telegram,
        PlatformKind::Slack,
        PlatformKind::Matrix,
    ] {
        for api in [false, true] {
            // Owner creation consumes three attempts; these two probes stay below six.
            let (app, directory) = fixture().await;
            let path = directory.path().join(".env");
            std::fs::write(
                &path,
                "DISCORD_TOKEN=settings-probe-saved-secret with-space\nTELEGRAM_TOKEN=settings-probe-saved-secret with-space\nSLACK_APP_TOKEN=settings-probe-saved-secret with-space\nSLACK_BOT_TOKEN=settings-probe-saved-secret with-space\nMATRIX_HOMESERVER=https://matrix.example.org\nMATRIX_USER=@bot:matrix.example.org\nMATRIX_ACCESS_TOKEN=settings-probe-saved-secret with-space\nMATRIX_PASSWORD=settings-probe-saved-password\n",
            )
            .unwrap();
            *app.config.lock() = config::Config::load(&path, &HashMap::new()).unwrap();
            let admin = owner(&app).await;
            platform_draft(&app, &admin.token, platform);
            let original = std::fs::read(&path).unwrap();
            let revision = app.config.lock().revision().to_owned();
            let draft_values = app.drafts.lock()[&wizard_key(&admin.token)].values.clone();
            let config_values: HashMap<_, _> = config::REGISTRY
                .iter()
                .map(|field| (field.key, app.config.lock().get(field.key).to_owned()))
                .collect();
            let draft_state = {
                let drafts = app.drafts.lock();
                let draft = &drafts[&wizard_key(&admin.token)];
                (
                    draft.platform_nonce,
                    draft.expires,
                    draft.revision.clone(),
                    serde_json::to_value(&draft.application).unwrap(),
                )
            };
            let router = routes::router(app.clone());
            for supplied in [true, false] {
                let mut input: HashMap<String, String> = HashMap::from([
                    ("csrf".into(), admin.info.csrf.clone()),
                    ("platform".into(), platform.as_str().into()),
                    (
                        "MATRIX_PASSWORD".into(),
                        "settings-probe-supplied-password".into(),
                    ),
                ]);
                for &key in routes::platform_credentials(platform) {
                    input.insert(
                        key.into(),
                        if supplied && key.contains("TOKEN") {
                            "settings-probe-supplied-secret\ninjected".into()
                        } else {
                            String::new()
                        },
                    );
                }
                let mut encoded = reqwest::Url::parse("http://localhost/").unwrap();
                encoded.query_pairs_mut().extend_pairs(&input);
                let body = if api {
                    serde_json::to_string(&input).unwrap()
                } else {
                    encoded.query().unwrap().to_owned()
                };
                let mut post = request(
                    "POST",
                    if api {
                        "/api/v1/settings/test-platform?lang=tr"
                    } else {
                        "/ui/settings/test-platform?lang=tr"
                    },
                    Some(&admin.token),
                    &body,
                );
                if api {
                    post.headers_mut()
                        .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
                }
                let response = router.clone().oneshot(post).await.unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "{platform:?} api={api} supplied={supplied}"
                );
                assert!(response.headers()[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with(if api { "application/json" } else { "text/html" }));
                let identifier = response.headers()["x-request-id"]
                    .to_str()
                    .unwrap()
                    .to_owned();
                let body = response_text(response).await;
                for secret in [
                    "settings-probe-saved-secret",
                    "settings-probe-supplied-secret",
                    "settings-probe-saved-password",
                    "settings-probe-supplied-password",
                ] {
                    assert!(!body.contains(secret));
                }
                if api {
                    let error: serde_json::Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(error["request_id"], identifier);
                    assert!(error["code"]
                        .as_str()
                        .is_some_and(|value| !value.is_empty()));
                    #[cfg(not(feature = "matrix"))]
                    if platform == PlatformKind::Matrix && !supplied {
                        assert_eq!(error["code"], "panel.config.matrix_feature_required");
                    }
                } else {
                    assert!(body.contains("/settings?lang=tr"));
                }
                assert_eq!(std::fs::read(&path).unwrap(), original);
                assert_eq!(app.config.lock().revision(), revision);
                for (&key, value) in &config_values {
                    assert_eq!(app.config.lock().get(key), value);
                }
                let drafts = app.drafts.lock();
                assert_eq!(drafts.len(), 1);
                let draft = &drafts[&wizard_key(&admin.token)];
                assert_eq!(draft.values, draft_values);
                assert_eq!(draft.selected_platform, platform);
                assert_eq!(draft.application.as_ref().unwrap().platform, platform);
                assert!(draft.provider_verified);
                assert!(draft.fit_completed);
                assert_eq!(draft.platform_attempt, 0);
                assert_eq!(draft.platform_nonce, draft_state.0);
                assert_eq!(draft.expires, draft_state.1);
                assert_eq!(draft.revision, draft_state.2);
                assert_eq!(
                    serde_json::to_value(&draft.application).unwrap(),
                    draft_state.3
                );
                assert!(matches!(
                    app.runtime.lock().lifecycle,
                    Lifecycle::Unconfigured
                ));
            }
        }
    }
}

#[tokio::test]
async fn settings_platform_tests_require_explicit_supported_platform_without_discord_fallback() {
    for api in [false, true] {
        let (app, directory) = fixture().await;
        settings_fixture(&app);
        let admin = owner(&app).await;
        let original = std::fs::read(directory.path().join(".env")).unwrap();
        let router = routes::router(app.clone());
        for platform in [None, Some("not-a-platform"), Some("terminal")] {
            let mut input: HashMap<String, String> = HashMap::from([
                ("csrf".into(), admin.info.csrf.clone()),
                (
                    "DISCORD_TOKEN".into(),
                    "settings-probe-secret\ninvalid".into(),
                ),
            ]);
            if let Some(platform) = platform {
                input.insert("platform".into(), platform.into());
            }
            let mut encoded = reqwest::Url::parse("http://localhost/").unwrap();
            encoded.query_pairs_mut().extend_pairs(&input);
            let body = if api {
                serde_json::to_string(&input).unwrap()
            } else {
                encoded.query().unwrap().to_owned()
            };
            let mut post = request(
                "POST",
                if api {
                    "/api/v1/settings/test-platform"
                } else {
                    "/ui/settings/test-platform"
                },
                Some(&admin.token),
                &body,
            );
            if api {
                post.headers_mut()
                    .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
            }
            let response = router.clone().oneshot(post).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            let body = response_text(response).await;
            assert!(!body.contains("settings-probe-secret"));
            if api {
                let error: serde_json::Value = serde_json::from_str(&body).unwrap();
                assert_eq!(error["code"], "validation_failed");
            }
        }
        assert!(app.drafts.lock().is_empty());
        assert_eq!(
            std::fs::read(directory.path().join(".env")).unwrap(),
            original
        );
    }
}

#[tokio::test]
async fn log_stream_ends_and_releases_capacity_when_the_session_is_revoked() {
    use futures_util::StreamExt;
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let response = routes::router(app.clone())
        .oneshot(request(
            "GET",
            "/api/v1/logs/stream",
            Some(&admin.token),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(app.streams.available_permits(), 7);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    app.auth.logout(&admin.token, &admin.info.csrf).unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(std::str::from_utf8(&ended)
        .unwrap()
        .contains("event: session_ended"));
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(app.streams.available_permits(), 8);
}

#[tokio::test]
async fn log_stream_connections_are_bounded_and_recent_logs_remain_readable_without_js() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    let mut streams = Vec::new();
    for _ in 0..8 {
        let response = router
            .clone()
            .oneshot(request(
                "GET",
                "/api/v1/logs/stream",
                Some(&admin.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        streams.push(response);
    }
    let response = router
        .clone()
        .oneshot(request(
            "GET",
            "/api/v1/logs/stream",
            Some(&admin.token),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    drop(streams);
    assert_eq!(app.streams.available_permits(), 8);
    let response = router
        .oneshot(request("GET", "/logs", Some(&admin.token), ""))
        .await
        .unwrap();
    let html = String::from_utf8(
        to_bytes(response.into_body(), 256 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains(views::text(views::Language::En, "panel.logs.body").unwrap()));
    assert!(html.contains(views::text(views::Language::En, "panel.logs.no_recent").unwrap()));
    assert!(html.contains("<noscript>"));
    assert!(html.contains("data-log-ended="));
}

#[tokio::test]
async fn login_explains_existing_ownership_and_nonsecret_settings_have_no_secret_metadata() {
    use crate::platform::types::PlatformKind;
    let (app, _directory) = fixture().await;
    settings_fixture(&app);
    let owner = owner(&app).await;
    let router = routes::router(app);
    let response = router
        .clone()
        .oneshot(request("GET", "/login", None, ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("Enter the administrator password you saved during setup"));
    assert!(!html.contains("Choose a unique admin password"));
    assert!(html.contains("nibby panel reset-password"));
    let response = router
        .clone()
        .oneshot(request("GET", "/settings", Some(&owner.token), ""))
        .await
        .unwrap();
    let html = String::from_utf8(
        to_bytes(response.into_body(), 256 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let probe_secret_fields = [
        PlatformKind::Discord,
        PlatformKind::Telegram,
        PlatformKind::Slack,
        PlatformKind::Matrix,
    ]
    .into_iter()
    .flat_map(|platform| routes::platform_credentials(platform).iter().copied())
    .filter(|key| {
        crate::config::REGISTRY
            .iter()
            .any(|spec| spec.key == *key && spec.secret)
    })
    .count();
    assert_eq!(
        html.matches("Secret configured:").count(),
        crate::config::REGISTRY
            .iter()
            .filter(|spec| spec.secret)
            .count()
            + probe_secret_fields
    );
    for field in html.split("<div class=\"field").skip(1) {
        let field = field.split("</div>").next().unwrap();
        if field.contains("Secret configured:") {
            let name = field
                .split(" name=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            assert!(crate::config::REGISTRY
                .iter()
                .any(|spec| spec.key == name && spec.secret));
        }
    }
    let response = router
        .oneshot(request("GET", "/setup?step=2", Some(&owner.token), ""))
        .await
        .unwrap();
    let html = String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("Leave Presence Intent off."));
}

#[tokio::test]
async fn numeric_loopback_health_needs_no_auth_or_host_override() {
    let (app, _directory) = fixture().await;
    let mut health = request("GET", "/healthz", None, "");
    health
        .headers_mut()
        .insert(header::HOST, "127.0.0.1:8787".parse().unwrap());
    let response = routes::router(app).oneshot(health).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(body, serde_json::json!({"status":"ok"}));
}

#[tokio::test]
async fn protected_settings_can_save_telegram_only_with_default_deny_without_starting_runtime() {
    let (app, directory) = fixture().await;
    let admin = owner(&app).await;
    let revision = app.config.lock().revision().to_owned();
    let body = format!(
        "csrf={}&revision={revision}&TELEGRAM_TOKEN=telegram-settings-private-credential&TELEGRAM_ROOMS=&OPENROUTER_KEY=provider-settings-private-credential",
        admin.info.csrf,
    );
    let router = routes::router(app.clone());
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            "/ui/settings?lang=tr",
            Some(&admin.token),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        "/settings?saved=1&lang=tr"
    );
    let saved = app.config.lock().reload().unwrap();
    saved.validate_runtime().unwrap();
    assert_eq!(saved.get("DISCORD_TOKEN"), "");
    assert_eq!(
        saved.get("TELEGRAM_TOKEN"),
        "telegram-settings-private-credential"
    );
    assert_eq!(saved.get("TELEGRAM_ROOMS"), "");
    assert!(
        crate::platform::telegram::runner::parse_rooms(saved.get("TELEGRAM_ROOMS"))
            .unwrap()
            .is_empty()
    );
    assert!(directory.path().join(".env").exists());
    assert_eq!(app.config.lock().get("TELEGRAM_TOKEN"), "");
    assert!(app
        .config
        .lock()
        .fields()
        .iter()
        .any(|view| view.spec.key == "TELEGRAM_TOKEN" && view.pending_restart));
    assert!(matches!(
        app.runtime.lock().lifecycle,
        Lifecycle::Unconfigured
    ));
    assert!(app.runtime.lock().bot.is_none());
    assert!(app.drafts.lock().is_empty());
    let response = router
        .oneshot(request("GET", "/settings?lang=tr", Some(&admin.token), ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(response.into_body(), 256 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(!html.contains("settings-private-credential"));
    assert!(html.contains("Onaylı Telegram odaları"));
    assert!(!html.contains("panel.settings.TELEGRAM"));
}

#[cfg(feature = "matrix")]
#[tokio::test]
async fn protected_matrix_settings_are_masked_and_pending_until_restart_without_starting_bot() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    let revision = app.config.lock().revision().to_owned();
    let body = format!(
        "csrf={}&revision={revision}&MATRIX_HOMESERVER=https%3A%2F%2Fmatrix.example&MATRIX_USER=%40bot%3Amatrix.example&MATRIX_ACCESS_TOKEN=matrix-settings-private-token&MATRIX_PASSWORD=matrix-settings-private-password&MATRIX_ROOMS=&OPENROUTER_KEY=provider-settings-private-key",
        admin.info.csrf,
    );
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            "/ui/settings?lang=tr",
            Some(&admin.token),
            &body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let saved = app.config.lock().reload().unwrap();
    saved.validate_runtime().unwrap();
    assert_eq!(saved.get("DISCORD_TOKEN"), "");
    assert_eq!(saved.get("MATRIX_HOMESERVER"), "https://matrix.example");
    assert_eq!(saved.get("MATRIX_USER"), "@bot:matrix.example");
    assert_eq!(
        saved.get("MATRIX_ACCESS_TOKEN"),
        "matrix-settings-private-token"
    );
    assert_eq!(
        saved.get("MATRIX_PASSWORD"),
        "matrix-settings-private-password"
    );
    assert_eq!(saved.get("MATRIX_ROOMS"), "");
    assert_eq!(app.config.lock().get("MATRIX_ACCESS_TOKEN"), "");
    assert!(matches!(
        app.runtime.lock().lifecycle,
        Lifecycle::Unconfigured
    ));
    assert!(app.runtime.lock().bot.is_none());
    assert!(app.drafts.lock().is_empty());
    let response = router
        .clone()
        .oneshot(request("GET", "/api/v1/settings", Some(&admin.token), ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 256 * 1024).await.unwrap()).unwrap();
    assert!(!value.to_string().contains("settings-private"));
    for key in ["MATRIX_ACCESS_TOKEN", "MATRIX_PASSWORD"] {
        let field = value["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["spec"]["key"] == key)
            .unwrap();
        assert_eq!(field["spec"]["kind"], "secret");
        assert_eq!(field["spec"]["application"], "restart");
        assert_eq!(field["saved"]["configured"], true);
        assert_eq!(field["pending_restart"], true);
        for source in ["effective", "saved", "default"] {
            assert!(field[source]["value"].is_null());
        }
    }
    for language in ["en", "tr"] {
        let response = router
            .clone()
            .oneshot(request(
                "GET",
                &format!("/settings?lang={language}"),
                Some(&admin.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(
            to_bytes(response.into_body(), 256 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(!html.contains("settings-private"));
        assert!(!html.contains("panel.settings.MATRIX"));
        assert!(html.contains(if language == "en" {
            "Approved Matrix rooms"
        } else {
            "Onaylı Matrix odaları"
        }));
    }
}

#[cfg(not(feature = "matrix"))]
#[tokio::test]
async fn disabled_matrix_config_remains_repairable_in_protected_settings_without_starting_bot() {
    let (app, _directory) = fixture().await;
    let path = app.config.lock().path().to_owned();
    std::fs::write(&path, "MATRIX_ACCESS_TOKEN=disabled-matrix-private-token\nMATRIX_PASSWORD=disabled-matrix-private-password\n").unwrap();
    *app.config.lock() = config::Config::load(path, &HashMap::new()).unwrap();
    assert_eq!(
        app.config.lock().validate_runtime().unwrap_err().code(),
        "panel.config.matrix_feature_required"
    );
    let admin = owner(&app).await;
    let router = routes::router(app.clone());
    for path in ["/settings?lang=en", "/settings?lang=tr", "/api/v1/settings"] {
        let response = router
            .clone()
            .oneshot(request("GET", path, Some(&admin.token), ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = String::from_utf8(
            to_bytes(response.into_body(), 256 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(!body.contains("disabled-matrix-private"));
        if path == "/api/v1/settings" {
            let settings: serde_json::Value = serde_json::from_str(&body).unwrap();
            let mut matrix_fields = 0;
            for field in settings["fields"].as_array().unwrap() {
                if field["spec"]["key"]
                    .as_str()
                    .unwrap()
                    .starts_with("MATRIX_")
                {
                    matrix_fields += 1;
                    for name in ["label_key", "help_key"] {
                        let key = field["spec"][name].as_str().unwrap();
                        for language in [views::Language::En, views::Language::Tr] {
                            assert_ne!(routes::t(language, key), key);
                        }
                    }
                }
            }
            assert_eq!(matrix_fields, 5);
        } else {
            assert!(!body.contains("panel.settings.MATRIX"));
        }
    }
    assert!(matches!(
        app.runtime.lock().lifecycle,
        Lifecycle::Unconfigured
    ));
    assert!(app.runtime.lock().bot.is_none());
}

#[tokio::test]
async fn unconfigured_usage_has_one_setup_card_and_no_fabricated_totals() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app);
    for language in [views::Language::En, views::Language::Tr] {
        let response = router
            .clone()
            .oneshot(request(
                "GET",
                &format!("/usage?lang={}", language.code()),
                Some(&admin.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = String::from_utf8(
            to_bytes(response.into_body(), 128 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert_eq!(body.matches("<section class=\"card\"").count(), 1);
        assert!(body.contains(views::text(language, "panel.overview.finish_setup").unwrap()));
        assert!(body.contains(&format!("href=\"/setup?lang={}\"", language.code())));
        assert!(!body.contains(views::text(language, "panel.usage.field_reported").unwrap()));
    }
}

#[tokio::test]
async fn editor_runtime_guard_prevents_cache_loading_between_selection_and_commit() {
    let (app, _directory) = fixture().await;
    let guard = routes::edit_runtime(&app).unwrap();
    assert_eq!(guard.lifecycle, Lifecycle::Unconfigured);
    assert!(app.runtime.try_lock().is_none());
    drop(guard);
    app.runtime.lock().lifecycle = Lifecycle::ConfiguredStarting;
    assert!(routes::edit_runtime(&app).is_err());
    app.runtime.lock().lifecycle = Lifecycle::Stopping;
    assert!(routes::edit_runtime(&app).is_err());
}

#[tokio::test]
async fn actions_keep_authenticated_navigation_in_both_languages() {
    let (app, _directory) = fixture().await;
    let admin = owner(&app).await;
    let router = routes::router(app);
    for language in [views::Language::En, views::Language::Tr] {
        let response = router
            .clone()
            .oneshot(request(
                "GET",
                &format!("/actions?lang={}", language.code()),
                Some(&admin.token),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert_eq!(body.matches("class=\"nav-link").count(), 9);
        assert_eq!(body.matches("class=\"nav-link nav-active\"").count(), 1);
    }
}

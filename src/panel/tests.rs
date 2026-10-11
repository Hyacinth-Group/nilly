use super::*;
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

async fn fixture() -> (Arc<App>, tempfile::TempDir) {
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
            origin: auth::OriginPolicy::new("http://localhost:8080").unwrap(),
            secure: false,
            config: Mutex::new(config),
            runtime: Mutex::new(Runtime {
                lifecycle: Lifecycle::Unconfigured,
                attempted: false,
                bot: None,
                manager: None,
                cache: None,
            }),
            drafts: Mutex::new(HashMap::new()),
            start,
            stop,
            tests: Arc::new(tokio::sync::Semaphore::new(2)),
            streams: Arc::new(tokio::sync::Semaphore::new(8)),
        }),
        directory,
    )
}

async fn owner(app: &App) -> auth::SessionIssue {
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

fn request(method: &str, path: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost:8080");
    if method != "GET" {
        request = request
            .header(header::ORIGIN, "http://localhost:8080")
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
        "/logs",
        "/account",
        "/help",
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
        .insert(header::HOST, "attacker.example:8080".parse().unwrap());
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
    draft.application = Some(setup::Application { id: "42".into(), name: "Verified test application".into(), invite_url: "https://discord.com/oauth2/authorize?client_id=42&permissions=67226688&scope=bot%20applications.commands".into() });
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
    assert!(body.contains("name=\"intents\" value=\"on\""));
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
    let body = format!("csrf={}&intents=on", admin.info.csrf);
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
            &format!("csrf={}&intents=on", admin.info.csrf),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!directory.path().join(".env").exists());
}

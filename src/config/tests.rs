use super::*;
use std::fs;

fn fixture(content: &str) -> (tempfile::TempDir, Config) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(".env");
    fs::write(&path, content).unwrap();
    let config = Config::load(path, &HashMap::new()).unwrap();
    (directory, config)
}

fn configured() -> &'static str {
    "# credentials\nDISCORD_TOKEN=token\nOPENROUTER_KEY=key\nBOT_LANG=en\n"
}

fn replace(key: &str, value: &str) -> Patch {
    HashMap::from([(
        key.to_owned(),
        PatchValue::Replace {
            value: value.to_owned(),
        },
    )])
}

fn save(config: &mut Config, patch: &Patch) -> ConfigResult<()> {
    let revision = config.revision().to_owned();
    config.save_patch(&revision, patch)
}

#[test]
fn registry_covers_every_example_and_panel_setting() {
    let example = Document::parse(include_str!("../../.env.example").to_owned(), true).unwrap();
    for key in example.values.keys() {
        assert!(registry::spec(key).is_some(), "{key}");
    }
    for key in [
        "PANEL_ENABLED",
        "PANEL_BIND",
        "PANEL_PORT",
        "PANEL_CONFIG_PATH",
        "PANEL_PUBLIC_URL",
        "PANEL_TRUSTED_PROXIES",
    ] {
        assert!(registry::spec(key).is_some());
    }
    let mut keys: Vec<_> = REGISTRY.iter().map(|field| field.key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), REGISTRY.len());
}

#[test]
fn telegram_rooms_use_runner_parser_and_blank_is_default_deny() {
    let (_directory, config) = fixture("TELEGRAM_TOKEN=telegram-secret\nOPENROUTER_KEY=key\n");
    assert_eq!(config.get("TELEGRAM_ROOMS"), "");
    assert!(
        crate::platform::telegram::runner::parse_rooms(config.get("TELEGRAM_ROOMS"))
            .unwrap()
            .is_empty()
    );
    config.validate_runtime().unwrap();
    assert_eq!(
        validate_value("TELEGRAM_ROOMS", "-100123, telegram:room:-77, -100123,42").unwrap(),
        "telegram:room:-100123,telegram:room:-77,telegram:room:42"
    );
    for value in [
        "0",
        "+42",
        "042",
        "-0",
        "telegram:room:0",
        "discord:room:42",
        "telegram:user:42",
        "telegram:room:01",
        "telegram:room:%2D42",
        "-42,,77",
        "9223372036854775808",
    ] {
        let error = validate_value("TELEGRAM_ROOMS", value).unwrap_err();
        assert_eq!(error.code(), "panel.config.invalid_telegram_rooms");
        assert!(!error.to_string().contains(value));
    }
    assert_eq!(validate_value("TELEGRAM_ROOMS", "  ").unwrap(), "");
}

#[test]
fn runtime_accepts_either_or_both_transports_and_preserves_provider_requirements() {
    for credentials in [
        "DISCORD_TOKEN=discord-secret\n",
        "TELEGRAM_TOKEN=telegram-secret\n",
        "DISCORD_TOKEN=discord-secret\nTELEGRAM_TOKEN=telegram-secret\n",
    ] {
        let (_directory, config) = fixture(&format!("{credentials}OPENROUTER_KEY=key\n"));
        config.validate_runtime().unwrap();
        let (_directory, config) = fixture(&format!("{credentials}MISTRAL_KEY=key\n"));
        config.validate_runtime().unwrap();
        let (_directory, config) = fixture(credentials);
        assert!(
            matches!(config.validate_runtime(), Err(ConfigError::Field { key, code }) if key == "OPENROUTER_KEY" && code == "panel.config.required")
        );
    }
    let (_directory, config) = fixture("OPENROUTER_KEY=key\n");
    assert!(
        matches!(config.validate_runtime(), Err(ConfigError::Field { key, code }) if key == "DISCORD_TOKEN" && code == "panel.config.platform_required")
    );
    let (_directory, config) =
        fixture("TELEGRAM_TOKEN=telegram-secret\nPROVIDER=mistral\nOPENROUTER_KEY=key\n");
    assert!(
        matches!(config.validate_runtime(), Err(ConfigError::Field { key, code }) if key == "MISTRAL_KEY" && code == "panel.config.required")
    );
}

#[test]
fn telegram_token_is_write_only_and_rooms_and_credentials_require_restart() {
    let (_directory, mut config) = fixture(
        "TELEGRAM_TOKEN=telegram-private-token\nTELEGRAM_ROOMS=-100123\nOPENROUTER_KEY=key\n",
    );
    let fields = config.fields();
    let token = fields
        .iter()
        .find(|field| field.spec.key == "TELEGRAM_TOKEN")
        .unwrap();
    assert!(token.spec.secret);
    assert!(matches!(token.spec.kind, FieldKind::Secret));
    assert_eq!(token.spec.application, "restart");
    assert!(token.effective.configured);
    assert!(token.effective.value.is_none());
    assert!(token.saved.value.is_none());
    assert!(token.default.value.is_none());
    assert!(!serde_json::to_string(&fields)
        .unwrap()
        .contains("telegram-private-token"));
    let rooms = fields
        .iter()
        .find(|field| field.spec.key == "TELEGRAM_ROOMS")
        .unwrap();
    assert!(matches!(rooms.spec.kind, FieldKind::TelegramRooms));
    assert_eq!(rooms.spec.application, "restart");
    assert_eq!(
        rooms.effective.value.as_deref(),
        Some("telegram:room:-100123")
    );
    save(&mut config, &replace("TELEGRAM_TOKEN", "")).unwrap();
    assert_eq!(
        config.reload().unwrap().get("TELEGRAM_TOKEN"),
        "telegram-private-token"
    );
    save(
        &mut config,
        &replace("TELEGRAM_TOKEN", "replacement-private-token"),
    )
    .unwrap();
    assert_eq!(config.get("TELEGRAM_TOKEN"), "telegram-private-token");
    assert_eq!(
        config.reload().unwrap().get("TELEGRAM_TOKEN"),
        "replacement-private-token"
    );
    assert!(config
        .fields()
        .iter()
        .any(|field| field.spec.key == "TELEGRAM_TOKEN" && field.pending_restart));
    save(&mut config, &replace("TELEGRAM_ROOMS", "")).unwrap();
    assert_eq!(config.get("TELEGRAM_ROOMS"), "telegram:room:-100123");
    assert_eq!(config.reload().unwrap().get("TELEGRAM_ROOMS"), "");
    assert!(config
        .fields()
        .iter()
        .any(|field| field.spec.key == "TELEGRAM_ROOMS" && field.pending_restart));
    assert!(
        matches!(save(&mut config, &HashMap::from([("TELEGRAM_TOKEN".into(), PatchValue::Clear)])), Err(ConfigError::Field { key, .. }) if key == "DISCORD_TOKEN")
    );
    save(
        &mut config,
        &replace("DISCORD_TOKEN", "discord-private-token"),
    )
    .unwrap();
    save(
        &mut config,
        &HashMap::from([("TELEGRAM_TOKEN".into(), PatchValue::Clear)]),
    )
    .unwrap();
    assert_eq!(config.reload().unwrap().get("TELEGRAM_TOKEN"), "");
    assert!(!serde_json::to_string(&config.fields())
        .unwrap()
        .contains("private-token"));
}

#[test]
fn telegram_environment_is_read_only_and_secrets_stay_absent_from_views() {
    let (directory, _) = fixture(
        "DISCORD_TOKEN=discord-secret\nTELEGRAM_TOKEN=file-telegram-private-token\nTELEGRAM_ROOMS=-77\nOPENROUTER_KEY=key\n",
    );
    let env = HashMap::from([
        (
            "TELEGRAM_TOKEN".into(),
            "environment-telegram-private-token".into(),
        ),
        ("TELEGRAM_ROOMS".into(), String::new()),
    ]);
    let mut config = Config::load(directory.path().join(".env"), &env).unwrap();
    assert_eq!(
        config.get("TELEGRAM_TOKEN"),
        "environment-telegram-private-token"
    );
    assert_eq!(config.get("TELEGRAM_ROOMS"), "");
    config.validate_runtime().unwrap();
    let fields = config.fields();
    for key in ["TELEGRAM_TOKEN", "TELEGRAM_ROOMS"] {
        let view = fields.iter().find(|view| view.spec.key == key).unwrap();
        assert!(view.read_only);
        assert!(matches!(view.source, Source::Environment));
        assert!(
            matches!(save(&mut config, &replace(key, "replacement")), Err(ConfigError::Field { key: rejected, code }) if rejected == key && code == "panel.config.read_only")
        );
    }
    assert!(!serde_json::to_string(&fields)
        .unwrap()
        .contains("private-token"));
    for value in ["secret\nSTEALTH=on", "secret\rtext", "secret\0text"] {
        let error = validate_value("TELEGRAM_TOKEN", value).unwrap_err();
        assert_eq!(error.code(), "panel.config.invalid_value");
        assert!(!error.to_string().contains(value));
    }
}

#[test]
fn qualified_owner_is_optional_and_uses_the_canonical_user_key_parser() {
    let field = registry::spec("OWNER_KEY").unwrap();
    assert!(matches!(field.kind, FieldKind::UserKey));
    assert!(!field.secret);
    assert_eq!(field.default, "");
    for value in [
        "discord:user:42",
        "telegram:user:42",
        "slack:user:T123:U456",
        "matrix:user:%40alice%3Aexample.org",
        "terminal:user:local:alice",
    ] {
        assert_eq!(validate_value("OWNER_KEY", value).unwrap(), value);
        let (_directory, config) =
            fixture(&format!("{}OWNER_KEY={value}\nOWNER_ID=42\n", configured()));
        config.validate_runtime().unwrap();
        assert_eq!(config.get("OWNER_KEY"), value);
    }
    assert_eq!(
        validate_value("OWNER_KEY", " telegram:user:42 ").unwrap(),
        "telegram:user:42"
    );
    for blank in ["", " \t "] {
        assert_eq!(validate_value("OWNER_KEY", blank).unwrap(), "");
        let (_directory, config) =
            fixture(&format!("{}OWNER_KEY={blank}\nOWNER_ID=42\n", configured()));
        config.validate_runtime().unwrap();
        assert_eq!(config.get("OWNER_KEY"), "");
        assert_eq!(config.get("OWNER_ID"), "42");
    }
}

#[test]
fn invalid_explicit_owner_is_rejected_without_legacy_fallback() {
    for value in [
        "42",
        "invalid",
        "telegram:room:42",
        "slack:user:U456",
        "matrix:user:@alice:example.org",
        "matrix:user:%40alice%3aexample.org",
    ] {
        assert!(matches!(
            validate_value("OWNER_KEY", value),
            Err(ConfigError::Field {
                key,
                code: "panel.config.invalid_user_key",
            }) if key == "OWNER_KEY"
        ));
        let (_directory, config) =
            fixture(&format!("{}OWNER_KEY={value}\nOWNER_ID=42\n", configured()));
        assert!(matches!(
            config.validate_runtime(),
            Err(ConfigError::Field {
                key,
                code: "panel.config.invalid_user_key",
            }) if key == "OWNER_KEY"
        ));
        let (_directory, mut config) = fixture(&format!("{}OWNER_ID=42\n", configured()));
        let original = fs::read_to_string(config.path()).unwrap();
        assert!(save(&mut config, &replace("OWNER_KEY", value)).is_err());
        assert_eq!(fs::read_to_string(config.path()).unwrap(), original);
    }
}

#[test]
fn qualified_owner_environment_precedence_is_immutable_and_read_only() {
    for value in ["telegram:user:42", "", "invalid"] {
        let (directory, _) = fixture(&format!(
            "{}OWNER_KEY=discord:user:42\nOWNER_ID=42\n",
            configured()
        ));
        let env = HashMap::from([("OWNER_KEY".to_owned(), value.to_owned())]);
        let mut config = Config::load(directory.path().join(".env"), &env).unwrap();
        assert_eq!(config.get("OWNER_KEY"), value);
        assert_eq!(config.get("OWNER_ID"), "42");
        let field = config
            .fields()
            .into_iter()
            .find(|field| field.spec.key == "OWNER_KEY")
            .unwrap();
        assert!(field.read_only);
        assert!(matches!(field.source, Source::Environment));
        assert_eq!(field.effective.value.as_deref(), Some(value));
        assert_eq!(field.saved.value.as_deref(), Some("discord:user:42"));
        assert!(matches!(
            save(&mut config, &replace("OWNER_KEY", "matrix:user:alice")),
            Err(ConfigError::Field {
                key,
                code: "panel.config.read_only",
            }) if key == "OWNER_KEY"
        ));
        if value == "invalid" {
            assert!(matches!(
                config.validate_runtime(),
                Err(ConfigError::Field { key, .. }) if key == "OWNER_KEY"
            ));
        } else {
            config.validate_runtime().unwrap();
        }
        fs::write(
            config.path(),
            format!("{}OWNER_KEY=matrix:user:alice\nOWNER_ID=99\n", configured()),
        )
        .unwrap();
        config.refresh_saved().unwrap();
        assert_eq!(config.get("OWNER_KEY"), value);
        assert_eq!(config.get("OWNER_ID"), "42");
        let field = config
            .fields()
            .into_iter()
            .find(|field| field.spec.key == "OWNER_KEY")
            .unwrap();
        assert_eq!(field.saved.value.as_deref(), Some("matrix:user:alice"));
        assert_eq!(config.reload().unwrap().get("OWNER_KEY"), value);
    }
}

#[test]
fn missing_file_supports_bootstrap_without_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::load(directory.path().join(".env"), &HashMap::new()).unwrap();
    assert_eq!(config.get("PANEL_BIND"), "127.0.0.1");
    assert_eq!(config.get("PANEL_PORT"), "8787");
    assert_eq!(config.get("PANEL_ENABLED"), "on");
    assert_eq!(config.get("BOT_LANG"), "en");
    assert!(
        matches!(config.validate_runtime(), Err(ConfigError::Field { key, .. }) if key == "DISCORD_TOKEN")
    );
}

#[test]
fn startup_environment_wins_even_when_empty_and_is_read_only() {
    let (directory, _) = fixture(configured());
    let env = HashMap::from([
        ("BOT_LANG".to_owned(), "tr".to_owned()),
        ("OPENROUTER_KEY".to_owned(), String::new()),
    ]);
    let mut config = Config::load(directory.path().join(".env"), &env).unwrap();
    assert_eq!(config.get("BOT_LANG"), "tr");
    assert_eq!(config.get("OPENROUTER_KEY"), "");
    assert!(
        matches!(config.validate_runtime(), Err(ConfigError::Field { key, .. }) if key == "OPENROUTER_KEY")
    );
    assert!(matches!(
        save(&mut config, &replace("BOT_LANG", "en")),
        Err(ConfigError::Field {
            code: "panel.config.read_only",
            ..
        })
    ));
    assert!(matches!(
        save(&mut config, &replace("OPENROUTER_KEY", "new")),
        Err(ConfigError::Field {
            code: "panel.config.read_only",
            ..
        })
    ));
    let field = config
        .fields()
        .into_iter()
        .find(|field| field.spec.key == "BOT_LANG")
        .unwrap();
    assert!(field.read_only);
    assert!(matches!(field.source, Source::Environment));
    assert_eq!(field.effective.value.as_deref(), Some("tr"));
    assert_eq!(field.saved.value.as_deref(), Some("en"));
}

#[test]
fn defaults_follow_provider_and_language_and_explicit_provider_wins() {
    let (_directory, config) = fixture("DISCORD_TOKEN=t\nMISTRAL_KEY=m\nBOT_LANG=tr\n");
    config.validate_runtime().unwrap();
    assert_eq!(config.selected_provider(), "mistral");
    assert_eq!(config.get("MODEL"), "mistral-medium-latest");
    assert_eq!(
        config.get("API_URL"),
        "https://api.mistral.ai/v1/chat/completions"
    );
    assert_eq!(
        config.get("NEWS_RSS"),
        "https://www.sozcu.com.tr/rss/news.xml"
    );
    let (_directory, config) = fixture("DISCORD_TOKEN=t\nOPENROUTER_KEY=o\nMISTRAL_KEY=m\n");
    assert_eq!(config.selected_provider(), "openrouter");
    let (_directory, config) = fixture("DISCORD_TOKEN=t\nMISTRAL_KEY=m\nPROVIDER=openrouter\n");
    assert!(config.validate_runtime().is_err());
}

#[test]
fn validation_rejects_invalid_values_without_disclosing_secrets() {
    for (key, value) in [
        ("DISCORD_TOKEN", "secret\nSTEALTH=on"),
        ("OPENROUTER_KEY", "secret\0text"),
        ("MISTRAL_KEY", "secret\rtext"),
        ("TZ_OFFSET", "NaN"),
        ("TZ_OFFSET", "inf"),
        ("TZ_OFFSET", "14.1"),
        ("TZ_OFFSET", "-12.1"),
        ("OWNER_ID", "0"),
        ("OWNER_ID", "18446744073709551616"),
        ("OWNER_ID", "+42"),
        ("CHANNELS", "1,,2"),
        ("BOT_LANG", "de"),
        ("PROVIDER", "unknown"),
        ("IMAGE_ANALYSIS", "maybe"),
        ("LOG_LEVEL", "verbose"),
        ("PANEL_PORT", "0"),
        ("PANEL_PORT", "65536"),
        ("PANEL_BIND", "host.example"),
        ("PANEL_TRUSTED_PROXIES", "127.0.0.1,broken"),
        ("MODEL", "model name"),
        ("API_URL", "https://user:password@example.com/completions"),
        ("NEWS_RSS", "file:///etc/passwd"),
        ("API_URL", "http://remote.example/completions"),
        ("PANEL_PUBLIC_URL", "https://example.com/subpath"),
        ("NEWS_RSS", "https://example.com/feed#fragment"),
    ] {
        let error = validate_value(key, value).unwrap_err();
        assert!(
            !error.to_string().contains(value),
            "secret/input leaked for {key}"
        );
    }
    assert_eq!(validate_value("TZ_OFFSET", "5.5").unwrap(), "5.5");
    assert_eq!(validate_value("TZ_OFFSET", "-12").unwrap(), "-12");
    assert_eq!(validate_value("TZ_OFFSET", "14").unwrap(), "14");
    assert!(validate_value("API_URL", "http://127.0.0.1:1234/v1/chat/completions").is_ok());
    assert!(validate_value("API_URL", "http://[::1]:1234/completions").is_ok());
    assert_eq!(validate_value("CHANNELS", "42, 43,42").unwrap(), "42,43");
}

#[test]
fn legacy_booleans_are_canonical() {
    for value in ["on", "1", "yes", "true", "acik", "açık", "evet", "ON"] {
        assert_eq!(validate_value("IMAGE_ANALYSIS", value).unwrap(), "on");
    }
    for value in [
        "off", "0", "no", "false", "kapali", "kapalı", "hayir", "hayır",
    ] {
        assert_eq!(validate_value("STEALTH", value).unwrap(), "off");
    }
    assert_eq!(validate_value("LOG_COLOR", "auto").unwrap(), "auto");
}

#[test]
fn preserves_comments_unknown_keys_duplicates_and_line_endings() {
    let original = "# private\r\nDISCORD_TOKEN=token\r\nOPENROUTER_KEY=key\r\nexport BOT_LANG = en # language\r\nCUSTOM='literal $x # unknown'\r\nBOT_LANG=en # duplicate\r\n# trailing\r\n";
    let (_directory, mut config) = fixture(original);
    assert_eq!(config.unknown_keys(), vec!["CUSTOM"]);
    save(&mut config, &replace("BOT_LANG", "tr")).unwrap();
    let saved = fs::read_to_string(config.path()).unwrap();
    assert!(saved.contains("# private\r\n"));
    assert!(saved.contains("export BOT_LANG =tr # language\r\n"));
    assert!(saved.contains("BOT_LANG=tr # duplicate\r\n"));
    assert!(saved.contains("CUSTOM='literal $x # unknown'\r\n"));
    assert!(saved.ends_with("# trailing\r\n"));
    assert_eq!(config.get("BOT_LANG"), "en");
    assert_eq!(config.reload().unwrap().get("BOT_LANG"), "tr");
    let field = config
        .fields()
        .into_iter()
        .find(|field| field.spec.key == "BOT_LANG")
        .unwrap();
    assert!(field.pending_restart);
}

#[test]
fn writer_roundtrips_quotes_backslashes_hash_dollars_and_unicode() {
    let (_directory, mut config) = fixture(configured());
    let secret = "new \"quoted\" \\key # $HOME 'şifre'";
    save(&mut config, &replace("OPENROUTER_KEY", secret)).unwrap();
    assert_eq!(config.reload().unwrap().get("OPENROUTER_KEY"), secret);
    // The emitted file also roundtrips with the legacy dotenv reader, without interpolation.
    let text = fs::read_to_string(config.path()).unwrap();
    let parsed: HashMap<_, _> = dotenvy::from_read_iter(text.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(parsed["OPENROUTER_KEY"], secret);
}

#[test]
fn secret_keep_replace_clear_masking_and_dependency_validation() {
    let (_directory, mut config) = fixture(configured());
    let before = fs::read_to_string(config.path()).unwrap();
    save(
        &mut config,
        &HashMap::from([("OPENROUTER_KEY".into(), PatchValue::Keep)]),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(config.path()).unwrap(), before);
    save(&mut config, &replace("OPENROUTER_KEY", "")).unwrap();
    assert_eq!(config.reload().unwrap().get("OPENROUTER_KEY"), "key");
    assert!(
        matches!(save(&mut config, &HashMap::from([("OPENROUTER_KEY".into(), PatchValue::Clear)])), Err(ConfigError::Field { key, .. }) if key == "OPENROUTER_KEY")
    );
    save(&mut config, &replace("FIRECRAWL_KEY", "optional-secret")).unwrap();
    let json = serde_json::to_string(&config.fields()).unwrap();
    assert!(!json.contains("optional-secret"));
    for field in config.fields().iter().filter(|field| field.spec.secret) {
        assert!(field.effective.value.is_none());
        assert!(field.saved.value.is_none());
        assert!(field.default.value.is_none());
    }
    save(
        &mut config,
        &HashMap::from([("FIRECRAWL_KEY".into(), PatchValue::Clear)]),
    )
    .unwrap();
    assert_eq!(config.reload().unwrap().get("FIRECRAWL_KEY"), "");
}

#[test]
fn rejects_injected_patch_without_modifying_file() {
    let (_directory, mut config) = fixture(configured());
    let before = fs::read_to_string(config.path()).unwrap();
    for value in ["key\nPANEL_ENABLED=off", "key\rOTHER=value", "key\0ignored"] {
        assert!(save(&mut config, &replace("OPENROUTER_KEY", value)).is_err());
        assert_eq!(fs::read_to_string(config.path()).unwrap(), before);
    }
    assert!(save(&mut config, &replace("UNKNOWN", "anything")).is_err());
    assert!(save(&mut config, &replace("PANEL_CONFIG_PATH", "/tmp/elsewhere")).is_err());
}

#[test]
fn stale_forms_and_external_file_changes_conflict() {
    let (_directory, mut first) = fixture(configured());
    let mut second = first.reload().unwrap();
    let old_revision = first.revision().to_owned();
    save(&mut first, &replace("BOT_LANG", "tr")).unwrap();
    assert!(matches!(
        second.save_patch(&old_revision, &replace("MODEL", "other")),
        Err(ConfigError::Conflict)
    ));
    assert!(matches!(
        first.save_patch(&old_revision, &replace("MODEL", "other")),
        Err(ConfigError::Conflict)
    ));
    let revision = first.revision().to_owned();
    fs::write(
        first.path(),
        format!(
            "{}# externally edited\n",
            fs::read_to_string(first.path()).unwrap()
        ),
    )
    .unwrap();
    assert!(matches!(
        first.save_patch(&revision, &replace("MODEL", "other")),
        Err(ConfigError::Conflict)
    ));
    assert!(fs::read_to_string(first.path())
        .unwrap()
        .ends_with("# externally edited\n"));
}

#[test]
fn missing_and_existing_empty_files_have_distinct_revisions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(".env");
    let mut config = Config::load(&path, &HashMap::new()).unwrap();
    fs::write(&path, "").unwrap();
    let patch = Config::patch_from_dotenv(configured()).unwrap();
    let revision = config.revision().to_owned();
    assert!(matches!(
        config.save_patch(&revision, &patch),
        Err(ConfigError::Conflict)
    ));
}

#[test]
fn invalid_saved_values_can_be_loaded_and_repaired() {
    let (_directory, mut config) =
        fixture("DISCORD_TOKEN=token\nOPENROUTER_KEY=key\nBOT_LANG=invalid\n");
    assert!(config.validate_runtime().is_err());
    save(&mut config, &replace("BOT_LANG", "tr")).unwrap();
    config.reload().unwrap().validate_runtime().unwrap();
    let (_directory, mut config) = fixture("DISCORD_TOKEN=token\nOPENROUTER_KEY=\"unterminated\n");
    assert!(config.validate_runtime().is_err());
    save(&mut config, &replace("OPENROUTER_KEY", "valid")).unwrap();
    config.reload().unwrap().validate_runtime().unwrap();
}

#[test]
fn new_install_can_validate_complete_patch_and_save_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(".env");
    let mut config = Config::load(&path, &HashMap::new()).unwrap();
    let patch = Config::patch_from_dotenv(configured()).unwrap();
    let candidate = config.validate_patch(&patch).unwrap();
    assert_eq!(candidate.get("DISCORD_TOKEN"), "token");
    save(&mut config, &patch).unwrap();
    config.reload().unwrap().validate_runtime().unwrap();
}

#[test]
fn failed_persistence_leaves_snapshot_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing-directory/.env");
    let mut config = Config::load(&path, &HashMap::new()).unwrap();
    let revision = config.revision().to_owned();
    let patch = Config::patch_from_dotenv(configured()).unwrap();
    assert!(matches!(save(&mut config, &patch), Err(ConfigError::Io(_))));
    assert_eq!(config.revision(), revision);
    assert!(!path.exists());
}

#[test]
fn bounds_are_explicit() {
    assert!(validate_value("DISCORD_TOKEN", &"x".repeat(MAX_VALUE_BYTES + 1)).is_err());
    assert!(matches!(
        Document::parse("x".repeat(MAX_FILE_BYTES + 1), true),
        Err(ConfigError::TooLarge)
    ));
}

#[test]
fn logging_off_and_legacy_aliases_are_valid() {
    for value in ["off", "OFF", "kapali", "kapalı", "KAPALI"] {
        assert_eq!(validate_value("LOG_LEVEL", value).unwrap(), "off");
    }
    let (_directory, config) =
        fixture("DISCORD_TOKEN=token\nOPENROUTER_KEY=key\nLOG_LEVEL=kapali\n");
    config.validate_runtime().unwrap();
    assert_eq!(config.get("LOG_LEVEL"), "off");
}

#[test]
fn refresh_saved_recovers_external_cas_without_changing_active_snapshot() {
    let (_directory, mut config) = fixture(configured());
    let revision = config.revision().to_owned();
    fs::write(
        config.path(),
        "DISCORD_TOKEN=token\nOPENROUTER_KEY=new-key\nBOT_LANG=tr\nEXTERNAL=kept\n",
    )
    .unwrap();
    assert!(matches!(
        config.save_patch(&revision, &replace("MODEL", "new-model")),
        Err(ConfigError::Conflict)
    ));
    config.refresh_saved().unwrap();
    assert_ne!(config.revision(), revision);
    assert_eq!(config.get("BOT_LANG"), "en");
    assert_eq!(config.get("OPENROUTER_KEY"), "key");
    assert_eq!(config.unknown_keys(), vec!["EXTERNAL"]);
    let field = config
        .fields()
        .into_iter()
        .find(|field| field.spec.key == "BOT_LANG")
        .unwrap();
    assert_eq!(field.effective.value.as_deref(), Some("en"));
    assert_eq!(field.saved.value.as_deref(), Some("tr"));
    assert!(matches!(field.source, Source::File));
    assert!(field.pending_restart);
    assert!(config.secret_values().contains(&"new-key"));
    save(&mut config, &replace("MODEL", "new-model")).unwrap();
    let restarted = config.reload().unwrap();
    assert_eq!(restarted.get("BOT_LANG"), "tr");
    assert_eq!(restarted.get("OPENROUTER_KEY"), "new-key");
    assert_eq!(restarted.get("MODEL"), "new-model");
}

#[test]
fn refresh_saved_tracks_invalid_content_deletion_and_recovery() {
    let (_directory, mut config) = fixture(configured());
    fs::write(
        config.path(),
        "DISCORD_TOKEN=token\nOPENROUTER_KEY=key\nBOT_LANG=\"unterminated\n",
    )
    .unwrap();
    config.refresh_saved().unwrap();
    assert_eq!(config.get("BOT_LANG"), "en");
    assert!(config.validate_patch(&replace("MODEL", "model")).is_err());
    save(&mut config, &replace("BOT_LANG", "tr")).unwrap();
    fs::remove_file(config.path()).unwrap();
    config.refresh_saved().unwrap();
    assert_eq!(config.get("DISCORD_TOKEN"), "token");
    assert!(
        config
            .fields()
            .into_iter()
            .find(|field| field.spec.key == "DISCORD_TOKEN")
            .unwrap()
            .pending_restart
    );
    save(
        &mut config,
        &Config::patch_from_dotenv(configured()).unwrap(),
    )
    .unwrap();
    config.reload().unwrap().validate_runtime().unwrap();
}

#[test]
fn refresh_read_failure_does_not_publish_partial_state() {
    let (_directory, mut config) = fixture(configured());
    let revision = config.revision().to_owned();
    fs::write(config.path(), [0xff]).unwrap();
    assert!(matches!(
        config.refresh_saved(),
        Err(ConfigError::InvalidEncoding)
    ));
    assert_eq!(config.revision(), revision);
    assert_eq!(config.get("BOT_LANG"), "en");
    assert_eq!(config.saved["BOT_LANG"], "en");
}

#[test]
fn simultaneous_saves_from_same_revision_have_one_winner() {
    let (_directory, config) = fixture(configured());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["model-a", "model-b"]
        .into_iter()
        .map(|model| {
            let mut config = config.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                save(&mut config, &replace("MODEL", model))
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ConfigError::Conflict)))
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn creation_and_replacement_are_private_from_atomic_tempfile() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(".env");
    let mut config = Config::load(&path, &HashMap::new()).unwrap();
    save(
        &mut config,
        &Config::patch_from_dotenv(configured()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    save(&mut config, &replace("BOT_LANG", "tr")).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

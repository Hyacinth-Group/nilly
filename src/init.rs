use super::*;
use base64::Engine;
use std::io::{self, BufRead, IsTerminal, Write};

// View Channels, Send Messages, Embed Links, Attach Files, Read Message History,
// Add Reactions and Change Nickname.
const INVITE_PERMISSIONS: u64 = 67226688;
const FREE_OPENROUTER_MODEL: &str = "google/gemma-4-31b-it:free";
const EXISTING_ENV_MESSAGE: &str = ".env already exists. Your existing settings were not changed. Open .env to edit them, or rerun discord-bot init --force to replace them.";

#[derive(Debug, PartialEq, Eq)]
enum SetupTarget {
    Discord(Option<u64>),
    Telegram,
    Slack,
    #[cfg(feature = "matrix")]
    Matrix,
}

pub fn run(args: &[String]) -> Result<(), BotError> {
    let path = std::path::Path::new(".env");
    if path.exists() && !args.iter().any(|arg| arg == "--force") {
        return Err(EXISTING_ENV_MESSAGE.into());
    }

    let stdin = io::stdin();
    let (content, target) = build_env(&mut stdin.lock(), stdin.is_terminal())?;
    let patch = config::Config::patch_from_dotenv(&content)?;
    let mut settings = config::Config::load(path, &std::collections::HashMap::new())?;
    let revision = settings.revision().to_owned();
    settings.save_patch(&revision, &patch)?;

    let steps = match target {
        SetupTarget::Discord(app_id) => next_steps(app_id),
        SetupTarget::Telegram => strings::t("init.telegram.next_steps").to_string(),
        SetupTarget::Slack => strings::t("init.slack.next_steps").to_string(),
        #[cfg(feature = "matrix")]
        SetupTarget::Matrix => strings::t("init.matrix.next_steps").to_string(),
    };
    println!("Wrote .env\n{steps}");
    Ok(())
}

fn ask<R: BufRead>(input: &mut R, prompt: &str) -> Result<String, BotError> {
    let mut stderr = io::stderr().lock();
    write!(stderr, "{prompt}")?;
    stderr.flush()?;
    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 {
        return Err("input ended".into());
    }
    Ok(answer.trim().to_string())
}

fn ask_required<R: BufRead>(input: &mut R, prompt: &str, hidden: bool) -> Result<String, BotError> {
    loop {
        let answer = if hidden {
            rpassword::prompt_password(prompt)?.trim().to_string()
        } else {
            ask(input, prompt)?
        };
        if !answer.is_empty() {
            return Ok(answer);
        }
    }
}

// Collect answers without touching the filesystem, so the same flow is testable.
fn build_env<R: BufRead>(
    input: &mut R,
    hidden_secrets: bool,
) -> Result<(String, SetupTarget), BotError> {
    let platform = loop {
        #[cfg(feature = "matrix")]
        let prompt = strings::t("init.platform");
        #[cfg(not(feature = "matrix"))]
        let prompt = strings::t("init.platform.no_matrix");
        match ask(input, prompt)?.as_str() {
            "" | "discord" => break SetupTarget::Discord(None),
            "telegram" => break SetupTarget::Telegram,
            "slack" => break SetupTarget::Slack,
            #[cfg(feature = "matrix")]
            "matrix" => break SetupTarget::Matrix,
            _ => continue,
        }
    };
    let secret_hint = if hidden_secrets {
        strings::t("init.secret.hidden")
    } else {
        strings::t("init.secret.piped")
    };
    let mut content = String::new();
    let target = match platform {
        SetupTarget::Discord(_) => {
            let token = ask_required(
                input,
                &format!(
                    "Discord bot token (required; Developer Portal -> Bot -> Reset Token; {secret_hint}): "
                ),
                hidden_secrets,
            )?;
            let token_value = config::encode_value("DISCORD_TOKEN", &token)?;
            content.push_str(&format!("DISCORD_TOKEN={token_value}\n"));
            SetupTarget::Discord(app_id_from_token(&token))
        }
        SetupTarget::Telegram => {
            eprintln!("{}", strings::t("init.telegram.setup"));
            let prompt = strings::t("init.telegram.token").replace(
                "{secret_hint}",
                strings::t(if hidden_secrets {
                    "init.input_hidden"
                } else {
                    "init.piped_input"
                }),
            );
            let token = ask_required(input, &prompt, hidden_secrets)?;
            let token_value = config::encode_value("TELEGRAM_TOKEN", &token)?;
            content.push_str(&format!("TELEGRAM_TOKEN={token_value}\n"));
            SetupTarget::Telegram
        }
        SetupTarget::Slack => {
            let app_token =
                ask_slack_token(input, "init.slack.app_token", "xapp-", hidden_secrets)?;
            let bot_token =
                ask_slack_token(input, "init.slack.bot_token", "xoxb-", hidden_secrets)?;
            let channels = loop {
                let answer = ask(input, strings::t("init.slack.channels"))?;
                if answer.is_empty()
                    || answer.split(',').all(|channel| {
                        let channel = channel.trim();
                        (channel.starts_with('C') || channel.starts_with('G'))
                            && channel.len() > 1
                            && channel.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    })
                {
                    let mut normalized = String::with_capacity(answer.len());
                    for (index, channel) in answer.split(',').enumerate() {
                        if index > 0 {
                            normalized.push(',');
                        }
                        normalized.push_str(channel.trim());
                    }
                    break normalized;
                }
            };
            let app_token = config::encode_value("SLACK_APP_TOKEN", &app_token)?;
            let bot_token = config::encode_value("SLACK_BOT_TOKEN", &bot_token)?;
            let channels = config::encode_value("SLACK_CHANNELS", &channels)?;
            content.push_str(&format!(
                "SLACK_APP_TOKEN={app_token}\nSLACK_BOT_TOKEN={bot_token}\nSLACK_CHANNELS={channels}\n"
            ));
            SetupTarget::Slack
        }
        #[cfg(feature = "matrix")]
        SetupTarget::Matrix => {
            content.push_str(&matrix_credentials(input, hidden_secrets, secret_hint)?);
            SetupTarget::Matrix
        }
    };
    let provider = loop {
        let answer = ask(
            input,
            "Model provider [openrouter/mistral] (Enter = openrouter): ",
        )?;
        match answer.as_str() {
            "" | "openrouter" => break "openrouter",
            "mistral" => break "mistral",
            _ => continue,
        }
    };
    let (key_name, provider_name, key_source, default_model) = if provider == "mistral" {
        (
            "MISTRAL_KEY",
            "Mistral",
            "https://console.mistral.ai/api-keys",
            MISTRAL_MODEL,
        )
    } else {
        (
            "OPENROUTER_KEY",
            "OpenRouter",
            "https://openrouter.ai/keys",
            OPENROUTER_MODEL,
        )
    };
    let key = ask_required(
        input,
        &format!("{provider_name} API key (required; create at {key_source}; {secret_hint}): "),
        hidden_secrets,
    )?;
    let model = ask(input, &model_prompt(provider, default_model))?;
    let language = loop {
        let answer = ask(
            input,
            "Bot language: English [en] / Turkish [tr] (Enter = en): ",
        )?;
        match answer.as_str() {
            "" | "en" => break "en",
            "tr" => break "tr",
            _ => continue,
        }
    };
    let (owner_name, owner) = match target {
        SetupTarget::Telegram => {
            let owner = loop {
                let answer = ask(input, strings::t("init.telegram.owner"))?;
                if answer.is_empty() || valid_telegram_owner(&answer) {
                    break answer;
                }
            };
            ("OWNER_KEY", owner)
        }
        SetupTarget::Slack => {
            use crate::platform::types::{PlatformKind, UserKey};
            let owner = loop {
                let answer = ask(input, strings::t("init.slack.owner"))?;
                if answer.is_empty()
                    || answer
                        .parse::<UserKey>()
                        .is_ok_and(|key| key.platform() == PlatformKind::Slack)
                {
                    break answer;
                }
            };
            ("OWNER_KEY", owner)
        }
        #[cfg(feature = "matrix")]
        SetupTarget::Matrix => {
            let owner = loop {
                let answer = ask(input, strings::t("init.matrix.owner"))?;
                if !answer.is_empty() && config::validate_value("MATRIX_USER", &answer).is_ok() {
                    break platform::types::UserKey::new(
                        platform::types::PlatformKind::Matrix,
                        &[&answer],
                    )?
                    .to_string();
                }
            };
            ("OWNER_KEY", owner)
        }
        SetupTarget::Discord(_) => {
            let owner = loop {
                let answer = ask(
                    input,
                    "Your Discord user ID (optional; e.g. 123456789012345678; enable Developer Mode, right-click your user and Copy User ID; owner can switch /model; Enter = skip): ",
                )?;
                if config::validate_value("OWNER_ID", &answer).is_ok() {
                    break answer;
                }
            };
            ("OWNER_ID", owner)
        }
    };
    let rooms = if target == SetupTarget::Telegram {
        loop {
            let answer = ask(input, strings::t("init.telegram.rooms"))?;
            if let Some(rooms) = telegram_rooms(&answer) {
                break rooms;
            }
        }
    } else {
        String::new()
    };
    let offset = loop {
        let answer = ask(
            input,
            "UTC offset in hours for the sleep schedule (e.g. 0, -5, 5.5; Enter = 0): ",
        )?;
        if sleep::parse_offset(&answer).is_some() {
            break if answer.is_empty() {
                "0".to_string()
            } else {
                answer
            };
        }
    };

    let key_value = config::encode_value(key_name, &key)?;
    let model = config::encode_value("MODEL", &model)?;
    content.push_str(&format!("{key_name}={key_value}\n"));
    content.push_str(&format!(
        "PROVIDER={provider}\nMODEL={model}\nBOT_LANG={language}\n"
    ));
    let owner = config::encode_value(owner_name, &owner)?;
    content.push_str(&format!("{owner_name}={owner}\n"));
    if target == SetupTarget::Telegram {
        let rooms = config::encode_value("TELEGRAM_ROOMS", &rooms)?;
        content.push_str(&format!("TELEGRAM_ROOMS={rooms}\n"));
    }
    content.push_str(&format!(
        "TZ_OFFSET={offset}\n# more options: see .env.example\n"
    ));
    Ok((content, target))
}

fn ask_slack_token<R: BufRead>(
    input: &mut R,
    prompt_key: &'static str,
    prefix: &str,
    hidden: bool,
) -> Result<String, BotError> {
    let secret_hint = strings::t(if hidden {
        "init.secret.hidden"
    } else {
        "init.secret.piped"
    });
    let prompt = strings::t(prompt_key).replace("{secret_hint}", secret_hint);
    loop {
        let token = ask_required(input, &prompt, hidden)?;
        if token.starts_with(prefix) && token.len() > prefix.len() {
            return Ok(token);
        }
    }
}

fn valid_telegram_owner(value: &str) -> bool {
    use crate::platform::types::{PlatformKind, UserKey};
    let Ok(key) = value.parse::<UserKey>() else {
        return false;
    };
    key.platform() == PlatformKind::Telegram
        && value
            .strip_prefix("telegram:user:")
            .filter(|id| !id.starts_with('0') && id.bytes().all(|byte| byte.is_ascii_digit()))
            .is_some_and(|id| id.parse::<u64>().is_ok())
}

fn telegram_rooms(value: &str) -> Option<String> {
    use crate::platform::telegram::runner::parse_rooms;
    if value.is_empty() {
        return Some(String::new());
    }
    for value in value.split(',').map(str::trim) {
        let native = if let Some(native) = value.strip_prefix("telegram:room:") {
            native
        } else {
            if value.parse::<i64>().ok()? >= 0 {
                return None;
            }
            value
        };
        let digits = native.strip_prefix('-').unwrap_or(native);
        if digits.is_empty()
            || digits.starts_with('0')
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
            || native.parse::<i64>().is_err()
        {
            return None;
        }
    }
    let mut rooms: Vec<_> = parse_rooms(value).ok()?.into_iter().collect();
    rooms.sort_unstable();
    let mut output = String::new();
    for room in rooms {
        if !output.is_empty() {
            output.push(',');
        }
        output.push_str(room.as_str());
    }
    Some(output)
}

#[cfg(feature = "matrix")]
fn matrix_credentials<R: BufRead>(
    input: &mut R,
    hidden_secrets: bool,
    secret_hint: &str,
) -> Result<String, BotError> {
    eprintln!("{}", strings::t("init.matrix.plaintext"));
    let homeserver = loop {
        let answer = ask(input, strings::t("init.matrix.homeserver"))?;
        if answer.is_empty() {
            continue;
        }
        if let Ok(value) = config::encode_value("MATRIX_HOMESERVER", &answer) {
            break value;
        }
    };
    let user = loop {
        let answer = ask(input, strings::t("init.matrix.user"))?;
        if answer.is_empty() {
            continue;
        }
        if let Ok(value) = config::encode_value("MATRIX_USER", &answer) {
            break value;
        }
    };
    let auth = loop {
        let answer = ask(input, strings::t("init.matrix.auth"))?;
        match answer.as_str() {
            "" | "token" => break "MATRIX_ACCESS_TOKEN",
            "password" => break "MATRIX_PASSWORD",
            _ => continue,
        }
    };
    let prompt = strings::t(if auth == "MATRIX_ACCESS_TOKEN" {
        "init.matrix.token"
    } else {
        "init.matrix.password"
    })
    .replace("{secret_hint}", secret_hint);
    let secret = config::encode_value(auth, &ask_required(input, &prompt, hidden_secrets)?)?;
    let rooms = loop {
        let answer = ask(input, strings::t("init.matrix.rooms"))?;
        if let Ok(value) = config::encode_value("MATRIX_ROOMS", &answer) {
            break value;
        }
    };
    Ok(format!(
        "MATRIX_HOMESERVER={homeserver}\nMATRIX_USER={user}\n{auth}={secret}\nMATRIX_ROOMS={rooms}\n"
    ))
}

fn model_prompt(provider: &str, default_model: &str) -> String {
    if provider == "openrouter" {
        format!("Model ID (Enter uses paid default {default_model}; free OpenRouter example: {FREE_OPENROUTER_MODEL}; free availability and rate limits may change): ")
    } else {
        format!("Model ID (e.g. {default_model}; Enter uses paid default {default_model}; free models require OpenRouter): ")
    }
}

fn next_steps(app_id: Option<u64>) -> String {
    let invite = match app_id {
        Some(id) => invite_url(id),
        None => format!(
            "Copy your application ID from Developer Portal -> General Information, replace <APP_ID>, then open:\nhttps://discord.com/oauth2/authorize?client_id=<APP_ID>&scope=bot+applications.commands&permissions={INVITE_PERMISSIONS}"
        ),
    };
    format!(
        "1. Enable Message Content Intent and Server Members Intent in Developer Portal -> Bot -> Privileged Gateway Intents. Leave Presence Intent off.\n\
         2. Start the bot: discord-bot (binary), cargo run --release (source), or docker compose up -d (Docker).\n\
         3. Invite the bot to your server by opening this link:\n{invite}\n\
         4. In Discord, run /status to confirm the bot responds (/durum if you selected Turkish)."
    )
}

pub fn app_id_from_token(token: &str) -> Option<u64> {
    let segment = token.split('.').next()?;
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let decoded = engine.decode(segment).ok()?;
    let digits = std::str::from_utf8(&decoded).ok()?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

pub fn invite_url(app_id: u64) -> String {
    format!("https://discord.com/oauth2/authorize?client_id={app_id}&scope=bot+applications.commands&permissions={INVITE_PERMISSIONS}")
}

#[cfg(test)]
mod test {
    use super::*;

    mod slack {
        use super::*;

        #[test]
        fn wizard_writes_slack_only_environment() {
            let mut input = io::Cursor::new(
                b"slack\nxapp-test\nxoxb-test\nC123, G456\n\nsk-test\n\ntr\nslack:user:T123:U456\n\n",
            );
            let (content, target) = build_env(&mut input, false).unwrap();
            assert_eq!(target, SetupTarget::Slack);
            let app_id = match target {
                SetupTarget::Discord(app_id) => app_id,
                SetupTarget::Telegram | SetupTarget::Slack => None,
                #[cfg(feature = "matrix")]
                SetupTarget::Matrix => None,
            };
            assert_eq!(app_id, None);
            assert!(!content.contains("TELEGRAM_TOKEN"));
            assert_eq!(
                content,
                "SLACK_APP_TOKEN=xapp-test\nSLACK_BOT_TOKEN=xoxb-test\nSLACK_CHANNELS=C123,G456\nOPENROUTER_KEY=sk-test\nPROVIDER=openrouter\nMODEL=\nBOT_LANG=tr\nOWNER_KEY=slack:user:T123:U456\nTZ_OFFSET=0\n# more options: see .env.example\n"
            );
            assert!(!content.contains("DISCORD_TOKEN"));
            assert!(!content.contains("OWNER_ID"));
        }

        #[test]
        fn wizard_encodes_slack_secrets_for_typed_config_save() {
            let mut input = io::Cursor::new(
                b"slack\nxapp-secret # comment\nxoxb-secret $HOME \"quoted\"\nC123, G456\n\nkey $HOME \"quoted\"\n\n\nslack:user:T123:U456\n\n",
            );
            let (content, target) = build_env(&mut input, false).unwrap();
            let patch = config::Config::patch_from_dotenv(&content).unwrap();
            assert_eq!(target, SetupTarget::Slack);
            for (name, expected) in [
                ("SLACK_APP_TOKEN", "xapp-secret # comment"),
                ("SLACK_BOT_TOKEN", "xoxb-secret $HOME \"quoted\""),
                ("OPENROUTER_KEY", "key $HOME \"quoted\""),
                ("SLACK_CHANNELS", "C123,G456"),
                ("OWNER_KEY", "slack:user:T123:U456"),
            ] {
                assert!(
                    matches!(&patch[name], config::PatchValue::Replace { value } if value == expected),
                    "{name}"
                );
            }
            assert!(!patch.contains_key("DISCORD_TOKEN"));
            assert!(!patch.contains_key("TELEGRAM_TOKEN"));
            assert!(!patch.contains_key("OWNER_ID"));
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(".env");
            let mut settings =
                config::Config::load(&path, &std::collections::HashMap::new()).unwrap();
            let revision = settings.revision().to_owned();
            settings.save_patch(&revision, &patch).unwrap();
            let saved = settings.reload().unwrap();
            assert_eq!(saved.get("SLACK_APP_TOKEN"), "xapp-secret # comment");
            assert_eq!(saved.get("SLACK_BOT_TOKEN"), "xoxb-secret $HOME \"quoted\"");
            assert_eq!(saved.get("OPENROUTER_KEY"), "key $HOME \"quoted\"");
            assert_eq!(saved.get("SLACK_CHANNELS"), "C123,G456");
            assert_eq!(saved.get("OWNER_KEY"), "slack:user:T123:U456");
            assert!(saved.get("DISCORD_TOKEN").is_empty());
            assert!(saved.get("TELEGRAM_TOKEN").is_empty());
        }

        #[test]
        fn wizard_rejects_null_slack_secret_before_serializing() {
            let mut input = io::Cursor::new(
                b"slack\nxapp-secret\0injected\nxoxb-secret\nC123\n\nkey\n\n\n\n\n",
            );
            assert!(build_env(&mut input, false).is_err());
        }

        #[test]
        fn wizard_reasks_invalid_slack_values_and_blank_channels_deny_all() {
            let mut input = io::Cursor::new(
                b"invalid\nslack\n\nxoxb-wrong\nxapp-test\nxapp-wrong\nxoxb-test\n#general\n\n\nsk-test\n\n\n42\ndiscord:user:42\nslack:user:T123:U456\n\n",
            );
            let (content, target) = build_env(&mut input, false).unwrap();
            assert_eq!(target, SetupTarget::Slack);
            assert!(content.contains("SLACK_CHANNELS=\n"));
            assert!(content.contains("OWNER_KEY=slack:user:T123:U456\n"));
        }

        #[test]
        fn manifest_parses_with_exact_scopes_and_subscriptions() {
            let manifest: serde_yaml::Value =
                serde_yaml::from_str(include_str!("../docs/slack-app-manifest.yml")).unwrap();
            let scopes = manifest["oauth_config"]["scopes"]["bot"]
                .as_sequence()
                .unwrap();
            let actual: Vec<_> = scopes.iter().map(|value| value.as_str().unwrap()).collect();
            assert_eq!(
                actual,
                [
                    "app_mentions:read",
                    "channels:history",
                    "channels:read",
                    "groups:history",
                    "groups:read",
                    "chat:write",
                    "files:read",
                    "files:write",
                    "reactions:read",
                    "reactions:write",
                    "users:read",
                ]
            );
            let events = manifest["settings"]["event_subscriptions"]["bot_events"]
                .as_sequence()
                .unwrap();
            let actual: Vec<_> = events.iter().map(|value| value.as_str().unwrap()).collect();
            assert_eq!(
                actual,
                [
                    "app_mention",
                    "message.channels",
                    "message.groups",
                    "member_joined_channel",
                    "reaction_added",
                ]
            );
            assert_eq!(
                manifest["settings"]["socket_mode_enabled"].as_bool(),
                Some(true)
            );
            assert!(manifest["features"]["slash_commands"].is_null());
            assert!(manifest["oauth_config"]["scopes"]["user"].is_null());
        }
    }

    #[test]
    fn app_id_from_token_decodes_digits() {
        assert_eq!(
            app_id_from_token("MTIzNDU2Nzg5MDEyMzQ1Njc4.x.y"),
            Some(123456789012345678)
        );
        assert_eq!(app_id_from_token("garbage"), None);
    }

    #[test]
    fn invite_url_has_required_scopes_and_permissions() {
        assert_eq!(
            invite_url(123456789012345678),
            "https://discord.com/oauth2/authorize?client_id=123456789012345678&scope=bot+applications.commands&permissions=67226688"
        );
    }

    #[test]
    fn build_env_collects_openrouter_answers() {
        let mut input =
            io::Cursor::new(b"\nMTIzNDU2Nzg5MDEyMzQ1Njc4.x.y\n\nsk-test\n\nen\n42\n-5\n");
        let (content, app_id) = build_env(&mut input, false).unwrap();
        assert!(matches!(app_id, SetupTarget::Discord(_)));
        assert!(content.contains("OPENROUTER_KEY=sk-test\n"));
        assert!(content.contains("OWNER_ID=42\n"));
        assert!(content.contains("TZ_OFFSET=-5\n"));
        assert!(content.contains("MODEL=\nBOT_LANG=en\n"));
        assert_eq!(app_id, SetupTarget::Discord(Some(123456789012345678)));
        assert!(content.ends_with("# more options: see .env.example\n"));
    }

    #[test]
    fn build_env_reasks_invalid_answers_and_supports_mistral() {
        let mut input = io::Cursor::new(
            b"unknown\ndiscord\n\ntoken\nunknown\nmistral\n\nsk-test\ncustom-model\nde\ntr\ninvalid\n42\n15\n5.5\n",
        );
        let (content, app_id) = build_env(&mut input, false).unwrap();
        assert!(matches!(app_id, SetupTarget::Discord(_)));
        assert_eq!(content, "DISCORD_TOKEN=token\nMISTRAL_KEY=sk-test\nPROVIDER=mistral\nMODEL=custom-model\nBOT_LANG=tr\nOWNER_ID=42\nTZ_OFFSET=5.5\n# more options: see .env.example\n");
        assert_eq!(app_id, SetupTarget::Discord(None));
    }

    #[test]
    fn build_env_defaults_optional_answers() {
        let mut input = io::Cursor::new(b"\ntoken\n\nsk-test\n\n\n\n\n");
        let (content, _) = build_env(&mut input, false).unwrap();
        assert!(content.contains("MODEL=\nBOT_LANG=en\nOWNER_ID=\nTZ_OFFSET=0\n"));
    }

    #[test]
    fn wizard_encodes_secrets_and_reasks_zero_owner() {
        let mut input =
            io::Cursor::new(b"\ntoken # comment\n\nkey $HOME \"quoted\"\n\n\n0\n42\n\n");
        let (content, _) = build_env(&mut input, false).unwrap();
        let patch = config::Config::patch_from_dotenv(&content).unwrap();
        assert!(
            matches!(&patch["DISCORD_TOKEN"], config::PatchValue::Replace { value } if value == "token # comment")
        );
        assert!(
            matches!(&patch["OPENROUTER_KEY"], config::PatchValue::Replace { value } if value == "key $HOME \"quoted\"")
        );
        assert!(
            matches!(&patch["OWNER_ID"], config::PatchValue::Replace { value } if value == "42")
        );
        assert!(content.contains("PROVIDER=openrouter\n"));
    }

    #[test]
    fn wizard_rejects_null_secret_before_serializing() {
        let mut input = io::Cursor::new(b"\ntoken\0injected\n\nkey\n\n\n\n\n");
        assert!(build_env(&mut input, false).is_err());
    }

    #[test]
    fn telegram_wizard_encodes_secrets_for_typed_config_save() {
        let mut input = io::Cursor::new(
            b"telegram\n123:secret # comment\n\nkey $HOME \"quoted\"\n\n\ntelegram:user:42\n-100123\n\n",
        );
        let (content, target) = build_env(&mut input, false).unwrap();
        let patch = config::Config::patch_from_dotenv(&content).unwrap();
        assert_eq!(target, SetupTarget::Telegram);
        assert!(
            matches!(&patch["TELEGRAM_TOKEN"], config::PatchValue::Replace { value } if value == "123:secret # comment")
        );
        assert!(
            matches!(&patch["OPENROUTER_KEY"], config::PatchValue::Replace { value } if value == "key $HOME \"quoted\"")
        );
        assert!(
            matches!(&patch["OWNER_KEY"], config::PatchValue::Replace { value } if value == "telegram:user:42")
        );
        assert!(
            matches!(&patch["TELEGRAM_ROOMS"], config::PatchValue::Replace { value } if value == "telegram:room:-100123")
        );
        assert!(!patch.contains_key("DISCORD_TOKEN"));
        assert!(!patch.contains_key("OWNER_ID"));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(".env");
        let mut settings = config::Config::load(&path, &std::collections::HashMap::new()).unwrap();
        let revision = settings.revision().to_owned();
        settings.save_patch(&revision, &patch).unwrap();
        let saved = settings.reload().unwrap();
        assert_eq!(saved.get("TELEGRAM_TOKEN"), "123:secret # comment");
        assert_eq!(saved.get("OPENROUTER_KEY"), "key $HOME \"quoted\"");
        assert_eq!(saved.get("OWNER_KEY"), "telegram:user:42");
        assert_eq!(saved.get("TELEGRAM_ROOMS"), "telegram:room:-100123");
        assert!(saved.get("DISCORD_TOKEN").is_empty());
    }

    #[test]
    fn build_env_keeps_explicit_free_model_and_trims_piped_secrets() {
        let answers = format!(
            "\r\n  token  \r\nopenrouter\r\n  sk-test  \r\n{FREE_OPENROUTER_MODEL}\r\n\r\n\r\n\r\n"
        );
        let (content, app_id) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert_eq!(
            content,
            format!("DISCORD_TOKEN=token\nOPENROUTER_KEY=sk-test\nPROVIDER=openrouter\nMODEL={FREE_OPENROUTER_MODEL}\nBOT_LANG=en\nOWNER_ID=\nTZ_OFFSET=0\n# more options: see .env.example\n")
        );
        assert_eq!(app_id, SetupTarget::Discord(None));
    }

    #[cfg(feature = "matrix")]
    #[test]
    fn build_env_matrix_prefers_token_and_encodes_owner() {
        let answers = b"matrix\nhttps://matrix.example.org\n@bot:example.org\n\nsyt_test\n!one:example.org,!two:example.org\n\nsk-test\n\nen\n@owner:example.org\n\n";
        let (content, target) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert_eq!(content, "MATRIX_HOMESERVER=https://matrix.example.org\nMATRIX_USER=\"@bot:example.org\"\nMATRIX_ACCESS_TOKEN=syt_test\nMATRIX_ROOMS=\"!one:example.org,!two:example.org\"\nOPENROUTER_KEY=sk-test\nPROVIDER=openrouter\nMODEL=\nBOT_LANG=en\nOWNER_KEY=\"matrix:user:%40owner%3Aexample.org\"\nTZ_OFFSET=0\n# more options: see .env.example\n");
        assert_eq!(target, SetupTarget::Matrix);
        assert!(!content.contains("DISCORD_TOKEN"));
        assert!(!content.contains("MATRIX_PASSWORD"));
        assert!(!content.contains("OWNER_ID"));
    }

    #[cfg(feature = "matrix")]
    #[test]
    fn build_env_matrix_password_reasks_invalid_configuration() {
        let answers = b"matrix\n\nmatrix.example.org\nftp://example.org\nhttps://example.org\nbot\n@:example.org\n@bot:\n@bot:example.org\nunknown\npassword\n\nsecret\n#alias:example.org\n!room:\n\nmistral\nsk-test\ncustom\ntr\n\nowner\n@owner:example.org\n5.5\n";
        let (content, _) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert!(content.contains("MATRIX_PASSWORD=secret\nMATRIX_ROOMS=\n"));
        assert!(content.contains("MISTRAL_KEY=sk-test\nPROVIDER=mistral\n"));
        assert!(content.contains("BOT_LANG=tr\nOWNER_KEY=\"matrix:user:%40owner%3Aexample.org\"\n"));
        assert!(content.contains("TZ_OFFSET=5.5\n"));
        assert!(!content.contains("MATRIX_ACCESS_TOKEN"));
    }

    #[cfg(feature = "matrix")]
    #[test]
    fn matrix_wizard_reasks_unsafe_homeservers_and_keeps_opaque_room_ids() {
        let answers = b"matrix\nhttps://user:secret@example.org\nhttps://example.org?token=secret\nhttps://example.org#secret\nhttps://example.org\n@bot:example.org\n\nsyt_test\n!opaqueRoomId, !legacy:example.org, !opaqueRoomId\n\nsk-test\n\n\n@owner:example.org\n\n";
        let (content, target) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert_eq!(target, SetupTarget::Matrix);
        assert!(content.contains("MATRIX_HOMESERVER=https://example.org\n"));
        assert!(content.contains("MATRIX_ROOMS=\"!opaqueRoomId,!legacy:example.org\"\n"));
        assert!(!content.contains("user:secret"));
        assert!(!content.contains("?token="));
        assert!(!content.contains("#secret"));
    }

    #[test]
    fn matrix_secret_survives_dotenv_quoting() {
        let secret = r#"pa ss#'$word"\end"#;
        let content = format!(
            "MATRIX_PASSWORD={}\n",
            config::encode_value("MATRIX_PASSWORD", secret).unwrap()
        );
        let entries = dotenvy::from_read_iter(content.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            entries,
            vec![("MATRIX_PASSWORD".to_string(), secret.to_string())]
        );
    }

    #[cfg(feature = "matrix")]
    #[test]
    fn matrix_wizard_encodes_secrets_for_typed_patch_save_and_reload() {
        let secret = r#"pa ss#'$word"\end"#;
        let answers = format!(
            "matrix\nhttps://matrix.example.org\n@bot:example.org\npassword\n{secret}\n!one:example.org, !two:example.org\n\nkey $HOME \"quoted\"\n\n\n@owner:example.org\n\n"
        );
        let (content, target) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert_eq!(target, SetupTarget::Matrix);
        let patch = config::Config::patch_from_dotenv(&content).unwrap();
        for (name, expected) in [
            ("MATRIX_HOMESERVER", "https://matrix.example.org"),
            ("MATRIX_USER", "@bot:example.org"),
            ("MATRIX_PASSWORD", secret),
            ("MATRIX_ROOMS", "!one:example.org,!two:example.org"),
            ("OPENROUTER_KEY", "key $HOME \"quoted\""),
            ("OWNER_KEY", "matrix:user:%40owner%3Aexample.org"),
        ] {
            assert!(
                matches!(&patch[name], config::PatchValue::Replace { value } if value == expected),
                "{name}"
            );
        }
        for name in [
            "DISCORD_TOKEN",
            "TELEGRAM_TOKEN",
            "SLACK_APP_TOKEN",
            "SLACK_BOT_TOKEN",
            "MATRIX_ACCESS_TOKEN",
            "OWNER_ID",
        ] {
            assert!(!patch.contains_key(name), "{name}");
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(".env");
        let mut settings = config::Config::load(&path, &std::collections::HashMap::new()).unwrap();
        let revision = settings.revision().to_owned();
        settings.save_patch(&revision, &patch).unwrap();
        let saved = settings.reload().unwrap();
        assert_eq!(saved.get("MATRIX_HOMESERVER"), "https://matrix.example.org");
        assert_eq!(saved.get("MATRIX_USER"), "@bot:example.org");
        assert_eq!(saved.get("MATRIX_PASSWORD"), secret);
        assert_eq!(
            saved.get("MATRIX_ROOMS"),
            "!one:example.org,!two:example.org"
        );
        assert_eq!(saved.get("OPENROUTER_KEY"), "key $HOME \"quoted\"");
        assert_eq!(saved.get("OWNER_KEY"), "matrix:user:%40owner%3Aexample.org");
        assert!(saved.get("MATRIX_ACCESS_TOKEN").is_empty());
        assert!(saved.get("DISCORD_TOKEN").is_empty());
        assert!(saved.get("TELEGRAM_TOKEN").is_empty());
        assert!(saved.get("SLACK_APP_TOKEN").is_empty());
        assert!(saved.get("SLACK_BOT_TOKEN").is_empty());
    }

    #[cfg(feature = "matrix")]
    #[test]
    fn matrix_wizard_rejects_null_secret_before_serializing() {
        let mut input = io::Cursor::new(
            b"matrix\nhttps://matrix.example.org\n@bot:example.org\n\nsecret\0injected\n",
        );
        assert!(build_env(&mut input, false).is_err());
    }

    #[cfg(not(feature = "matrix"))]
    #[test]
    fn wizard_without_matrix_feature_reasks_matrix_choice() {
        let mut input = io::Cursor::new(b"matrix\ndiscord\ntoken\n\nkey\n\n\n\n\n");
        let (content, target) = build_env(&mut input, false).unwrap();
        assert_eq!(target, SetupTarget::Discord(None));
        assert!(content.starts_with("DISCORD_TOKEN=token\n"));
        assert!(!content.contains("MATRIX_"));
        assert!(!strings::t("init.platform.no_matrix").contains("matrix"));
    }

    #[test]
    fn model_prompts_explain_paid_defaults_and_free_choice() {
        let openrouter = model_prompt("openrouter", OPENROUTER_MODEL);
        assert!(openrouter.contains(&format!("Enter uses paid default {OPENROUTER_MODEL}")));
        assert!(openrouter.contains(&format!("free OpenRouter example: {FREE_OPENROUTER_MODEL}")));
        assert!(openrouter.contains("availability and rate limits may change"));
        let mistral = model_prompt("mistral", MISTRAL_MODEL);
        assert!(mistral.contains(&format!("Enter uses paid default {MISTRAL_MODEL}")));
        assert!(mistral.contains("free models require OpenRouter"));
        assert!(!mistral.contains(FREE_OPENROUTER_MODEL));
    }

    #[test]
    fn build_env_telegram_runs_without_discord_credentials() {
        let mut input = io::Cursor::new(
            b"telegram\n123:secret\n\nsk-test\n\ntr\n42\ndiscord:user:42\ntelegram:user:42\n-100123, telegram:room:-456, -100123\n3\n",
        );
        let (content, target) = build_env(&mut input, false).unwrap();
        assert_eq!(target, SetupTarget::Telegram);
        assert_eq!(content, "TELEGRAM_TOKEN=123:secret\nOPENROUTER_KEY=sk-test\nPROVIDER=openrouter\nMODEL=\nBOT_LANG=tr\nOWNER_KEY=telegram:user:42\nTELEGRAM_ROOMS=telegram:room:-100123,telegram:room:-456\nTZ_OFFSET=3\n# more options: see .env.example\n");
        assert!(!content.contains("DISCORD_TOKEN"));
        assert!(!content.contains("OWNER_ID"));
    }

    #[test]
    fn telegram_defaults_deny_rooms_and_reasks_invalid_approval() {
        let mut input = io::Cursor::new(
            b"telegram\n123:secret\n\nsk-test\n\n\n\n123\n-01\ndiscord:room:42\n\n\n",
        );
        let (content, _) = build_env(&mut input, false).unwrap();
        assert!(content.contains("OWNER_KEY=\nTELEGRAM_ROOMS=\nTZ_OFFSET=0\n"));
    }

    #[test]
    fn telegram_identity_answers_are_native_and_canonical() {
        assert!(valid_telegram_owner("telegram:user:42"));
        for invalid in [
            "42",
            "telegram:user:0",
            "telegram:user:-42",
            "telegram:user:042",
            "telegram:user:+42",
            "telegram:user:name",
            "discord:user:42",
            "telegram:user:18446744073709551616",
        ] {
            assert!(!valid_telegram_owner(invalid), "{invalid}");
        }
        assert_eq!(
            telegram_rooms("-1001, telegram:room:-2, telegram:room:42"),
            Some("telegram:room:-1001,telegram:room:-2,telegram:room:42".to_string())
        );
        for invalid in [
            "0",
            "42",
            "-01",
            "-1001,",
            "telegram:room:0",
            "telegram:room:+42",
            "telegram:room:-01",
            "telegram:room:name",
            "telegram:room:-9223372036854775809",
            "discord:room:42",
        ] {
            assert_eq!(telegram_rooms(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn telegram_setup_strings_cover_safe_onboarding() {
        let setup = strings::t("init.telegram.setup");
        assert!(setup.contains("/newbot"));
        assert!(setup.contains("/setprivacy"));
        assert!(setup.contains("Disable"));
        let steps = strings::t("init.telegram.next_steps");
        assert!(steps.contains("TELEGRAM_ROOMS"));
        assert!(steps.contains("OWNER_KEY"));
        assert!(!steps.contains("discord.com/oauth2"));
    }

    #[test]
    fn telegram_onboarding_and_notices_exist_in_both_languages() {
        for raw in [
            include_str!("../langs/en.json"),
            include_str!("../langs/tr.json"),
        ] {
            let table: std::collections::HashMap<String, String> =
                serde_json::from_str(raw).unwrap();
            for key in [
                "init.platform",
                "init.telegram.setup",
                "init.telegram.token",
                "init.telegram.owner",
                "init.telegram.rooms",
                "init.telegram.next_steps",
                "telegram.privacy_warning",
                "telegram.nickname_warning",
                "telegram.history_warning",
                "telegram.commands.private",
                "telegram.startup.missing",
            ] {
                assert!(!table.get(key).unwrap().is_empty(), "{key}");
            }
            assert!(table["init.telegram.token"].contains("{secret_hint}"));
            assert!(table["init.telegram.setup"].contains("/setprivacy"));
            assert!(table["init.telegram.setup"].contains("/newbot"));
        }
    }

    #[test]
    fn next_steps_order_intents_start_invite_and_status() {
        let steps = next_steps(Some(123456789012345678));
        let lines: Vec<_> = steps.lines().collect();
        assert!(lines[0].starts_with("1. Enable Message Content Intent and Server Members Intent"));
        assert!(lines[0].contains("Leave Presence Intent off"));
        assert!(lines[1].starts_with("2. Start the bot:"));
        assert!(lines[2].starts_with("3. Invite the bot"));
        assert_eq!(lines[3], invite_url(123456789012345678));
        assert!(lines[4].starts_with("4. In Discord, run /status"));
    }

    #[test]
    fn next_steps_explain_application_id_fallback() {
        let steps = next_steps(None);
        assert!(steps.contains("Developer Portal -> General Information"));
        assert!(steps.contains("replace <APP_ID>"));
        assert!(steps.contains("client_id=<APP_ID>&scope=bot+applications.commands"));
        assert!(steps.ends_with("4. In Discord, run /status to confirm the bot responds (/durum if you selected Turkish)."));
    }

    #[test]
    fn existing_env_message_explains_preservation_editing_and_replacement() {
        assert!(EXISTING_ENV_MESSAGE.contains("Your existing settings were not changed"));
        assert!(EXISTING_ENV_MESSAGE.contains("Open .env to edit them"));
        assert!(EXISTING_ENV_MESSAGE.contains("discord-bot init --force to replace them"));
    }

    #[test]
    fn piped_required_secret_reports_eof_after_blank_answer() {
        let error = ask_required(&mut io::Cursor::new(b"\n"), "API key: ", false).unwrap_err();
        assert_eq!(error.to_string(), "input ended");
    }

    #[test]
    fn ask_reports_eof() {
        let error = ask(&mut io::Cursor::new(b""), "prompt: ").unwrap_err();
        assert_eq!(error.to_string(), "input ended");
    }
}

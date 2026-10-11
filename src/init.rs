use super::*;
use base64::Engine;
use std::io::{self, BufRead, IsTerminal, Write};

// View Channels, Send Messages, Embed Links, Attach Files, Read Message History,
// Add Reactions and Change Nickname.
const INVITE_PERMISSIONS: u64 = 67226688;
const FREE_OPENROUTER_MODEL: &str = "google/gemma-4-31b-it:free";
const EXISTING_ENV_MESSAGE: &str = ".env already exists. Your existing settings were not changed. Open .env to edit them, or rerun discord-bot init --force to replace them.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetupPlatform {
    Discord,
    Slack,
}

pub fn run(args: &[String]) -> Result<(), BotError> {
    let path = std::path::Path::new(".env");
    if path.exists() && !args.iter().any(|arg| arg == "--force") {
        return Err(EXISTING_ENV_MESSAGE.into());
    }

    let stdin = io::stdin();
    let (content, platform, app_id) = build_env(&mut stdin.lock(), stdin.is_terminal())?;
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    let steps = match platform {
        SetupPlatform::Discord => next_steps(app_id),
        SetupPlatform::Slack => strings::t("init.slack.next_steps").to_string(),
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
) -> Result<(String, SetupPlatform, Option<u64>), BotError> {
    let platform = loop {
        match ask(input, strings::t("init.platform"))?.as_str() {
            "" | "discord" => break SetupPlatform::Discord,
            "slack" => break SetupPlatform::Slack,
            _ => continue,
        }
    };
    let secret_hint = if hidden_secrets {
        strings::t("init.secret.hidden")
    } else {
        strings::t("init.secret.piped")
    };
    let mut content = String::new();
    let app_id = match platform {
        SetupPlatform::Discord => {
            let token = ask_required(
                input,
                &format!(
                    "Discord bot token (required; Developer Portal -> Bot -> Reset Token; {secret_hint}): "
                ),
                hidden_secrets,
            )?;
            content.push_str(&format!("DISCORD_TOKEN={token}\n"));
            app_id_from_token(&token)
        }
        SetupPlatform::Slack => {
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
            content.push_str(&format!(
                "SLACK_APP_TOKEN={app_token}\nSLACK_BOT_TOKEN={bot_token}\nSLACK_CHANNELS={channels}\n"
            ));
            None
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
    let (owner_name, owner) = match platform {
        SetupPlatform::Discord => {
            let owner = loop {
                let answer = ask(
                    input,
                    "Your Discord user ID (optional; e.g. 123456789012345678; enable Developer Mode, right-click your user and Copy User ID; owner can switch /model; Enter = skip): ",
                )?;
                if answer.bytes().all(|byte| byte.is_ascii_digit()) {
                    break answer;
                }
            };
            ("OWNER_ID", owner)
        }
        SetupPlatform::Slack => {
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

    content.push_str(&format!("{key_name}={key}\n"));
    if provider == "mistral" {
        content.push_str("PROVIDER=mistral\n");
    }
    content.push_str(&format!(
        "MODEL={model}\nBOT_LANG={language}\n{owner_name}={owner}\nTZ_OFFSET={offset}\n# more options: see .env.example\n"
    ));
    Ok((content, platform, app_id))
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
            let (content, platform, app_id) = build_env(&mut input, false).unwrap();
            assert_eq!(platform, SetupPlatform::Slack);
            assert_eq!(app_id, None);
            assert_eq!(
                content,
                "SLACK_APP_TOKEN=xapp-test\nSLACK_BOT_TOKEN=xoxb-test\nSLACK_CHANNELS=C123,G456\nOPENROUTER_KEY=sk-test\nMODEL=\nBOT_LANG=tr\nOWNER_KEY=slack:user:T123:U456\nTZ_OFFSET=0\n# more options: see .env.example\n"
            );
            assert!(!content.contains("DISCORD_TOKEN"));
            assert!(!content.contains("OWNER_ID"));
        }

        #[test]
        fn wizard_reasks_invalid_slack_values_and_blank_channels_deny_all() {
            let mut input = io::Cursor::new(
                b"invalid\nslack\n\nxoxb-wrong\nxapp-test\nxapp-wrong\nxoxb-test\n#general\n\n\nsk-test\n\n\n42\ndiscord:user:42\nslack:user:T123:U456\n\n",
            );
            let (content, platform, _) = build_env(&mut input, false).unwrap();
            assert_eq!(platform, SetupPlatform::Slack);
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
        let (content, platform, app_id) = build_env(&mut input, false).unwrap();
        assert_eq!(platform, SetupPlatform::Discord);
        assert!(content.contains("OPENROUTER_KEY=sk-test\n"));
        assert!(content.contains("OWNER_ID=42\n"));
        assert!(content.contains("TZ_OFFSET=-5\n"));
        assert!(content.contains("MODEL=\nBOT_LANG=en\n"));
        assert_eq!(app_id, Some(123456789012345678));
        assert!(content.ends_with("# more options: see .env.example\n"));
    }

    #[test]
    fn build_env_reasks_invalid_answers_and_supports_mistral() {
        let mut input = io::Cursor::new(
            b"\n\ntoken\nunknown\nmistral\n\nsk-test\ncustom-model\nde\ntr\ninvalid\n42\n15\n5.5\n",
        );
        let (content, platform, app_id) = build_env(&mut input, false).unwrap();
        assert_eq!(platform, SetupPlatform::Discord);
        assert_eq!(content, "DISCORD_TOKEN=token\nMISTRAL_KEY=sk-test\nPROVIDER=mistral\nMODEL=custom-model\nBOT_LANG=tr\nOWNER_ID=42\nTZ_OFFSET=5.5\n# more options: see .env.example\n");
        assert_eq!(app_id, None);
    }

    #[test]
    fn build_env_defaults_optional_answers() {
        let mut input = io::Cursor::new(b"\ntoken\n\nsk-test\n\n\n\n\n");
        let (content, _, _) = build_env(&mut input, false).unwrap();
        assert!(content.contains("MODEL=\nBOT_LANG=en\nOWNER_ID=\nTZ_OFFSET=0\n"));
    }

    #[test]
    fn build_env_keeps_explicit_free_model_and_trims_piped_secrets() {
        let answers = format!(
            "\r\n  token  \r\nopenrouter\r\n  sk-test  \r\n{FREE_OPENROUTER_MODEL}\r\n\r\n\r\n\r\n"
        );
        let (content, _, app_id) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert_eq!(
            content,
            format!("DISCORD_TOKEN=token\nOPENROUTER_KEY=sk-test\nMODEL={FREE_OPENROUTER_MODEL}\nBOT_LANG=en\nOWNER_ID=\nTZ_OFFSET=0\n# more options: see .env.example\n")
        );
        assert_eq!(app_id, None);
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

use super::*;
use base64::Engine;
use std::io::{self, BufRead, IsTerminal, Write};

// View Channels, Send Messages, Embed Links, Attach Files, Read Message History,
// Add Reactions and Change Nickname.
const INVITE_PERMISSIONS: u64 = 67226688;
const FREE_OPENROUTER_MODEL: &str = "google/gemma-4-31b-it:free";
const EXISTING_ENV_MESSAGE: &str = ".env already exists. Your existing settings were not changed. Open .env to edit them, or rerun discord-bot init --force to replace them.";

pub fn run(args: &[String]) -> Result<(), BotError> {
    let path = std::path::Path::new(".env");
    if path.exists() && !args.iter().any(|arg| arg == "--force") {
        return Err(EXISTING_ENV_MESSAGE.into());
    }

    let stdin = io::stdin();
    let (content, app_id) = build_env(&mut stdin.lock(), stdin.is_terminal())?;
    std::fs::write(path, &content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    let steps = if content.starts_with("MATRIX_HOMESERVER=") {
        strings::t("init.matrix.next_steps").to_string()
    } else {
        next_steps(app_id)
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
) -> Result<(String, Option<u64>), BotError> {
    let secret_hint = if hidden_secrets {
        "input hidden"
    } else {
        "piped input"
    };
    let matrix = loop {
        let answer = ask(input, strings::t("init.platform"))?;
        match answer.as_str() {
            "" | "discord" => break false,
            "matrix" => break true,
            _ => continue,
        }
    };
    let (mut content, app_id) = if matrix {
        (
            matrix_credentials(input, hidden_secrets, secret_hint)?,
            None,
        )
    } else {
        let token = ask_required(
            input,
            &format!(
                "Discord bot token (required; Developer Portal -> Bot -> Reset Token; {secret_hint}): "
            ),
            hidden_secrets,
        )?;
        (
            format!("DISCORD_TOKEN={token}\n"),
            app_id_from_token(&token),
        )
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
    let owner_setting = if matrix {
        let owner = loop {
            let answer = ask(input, strings::t("init.matrix.owner"))?;
            if valid_matrix_id(&answer, '@') {
                break platform::types::UserKey::new(
                    platform::types::PlatformKind::Matrix,
                    &[&answer],
                )?;
            }
        };
        format!("OWNER_KEY={owner}")
    } else {
        let owner = loop {
            let answer = ask(
                input,
                "Your Discord user ID (optional; e.g. 123456789012345678; enable Developer Mode, right-click your user and Copy User ID; owner can switch /model; Enter = skip): ",
            )?;
            if answer.bytes().all(|byte| byte.is_ascii_digit()) {
                break answer;
            }
        };
        format!("OWNER_ID={owner}")
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
        "MODEL={model}\nBOT_LANG={language}\n{owner_setting}\nTZ_OFFSET={offset}\n# more options: see .env.example\n"
    ));
    Ok((content, app_id))
}

fn valid_matrix_id(value: &str, sigil: char) -> bool {
    value.starts_with(sigil)
        && value.len() > 1
        && !value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
        && ((sigil == '!' && !value.contains(':'))
            || value[1..]
                .split_once(':')
                .is_some_and(|(local, server)| !local.is_empty() && !server.is_empty()))
}

fn matrix_credentials<R: BufRead>(
    input: &mut R,
    hidden_secrets: bool,
    secret_hint: &str,
) -> Result<String, BotError> {
    eprintln!("{}", strings::t("init.matrix.plaintext"));
    let homeserver = loop {
        let answer = ask(input, strings::t("init.matrix.homeserver"))?;
        if reqwest::Url::parse(&answer).is_ok_and(|url| {
            matches!(url.scheme(), "https" | "http")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        }) {
            break dotenv_value(answer);
        }
    };
    let user = loop {
        let answer = ask(input, strings::t("init.matrix.user"))?;
        if valid_matrix_id(&answer, '@') {
            break dotenv_value(answer);
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
    let secret = dotenv_value(ask_required(input, &prompt, hidden_secrets)?);
    let rooms = loop {
        let mut answer = ask(input, strings::t("init.matrix.rooms"))?;
        if answer.is_empty()
            || answer
                .split(',')
                .all(|room| valid_matrix_id(room.trim(), '!'))
        {
            answer.retain(|ch| !ch.is_whitespace());
            break dotenv_value(answer);
        }
    };
    Ok(format!(
        "MATRIX_HOMESERVER={homeserver}\nMATRIX_USER={user}\n{auth}={secret}\nMATRIX_ROOMS={rooms}\n"
    ))
}

fn dotenv_value(value: String) -> String {
    if value
        .chars()
        .all(|ch| !ch.is_whitespace() && !matches!(ch, '#' | '\'' | '"' | '$' | '\\'))
    {
        return value;
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        if matches!(ch, '\\' | '"' | '$') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
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
            b"unknown\ndiscord\n\ntoken\nunknown\nmistral\n\nsk-test\ncustom-model\nde\ntr\ninvalid\n42\n15\n5.5\n",
        );
        let (content, app_id) = build_env(&mut input, false).unwrap();
        assert_eq!(content, "DISCORD_TOKEN=token\nMISTRAL_KEY=sk-test\nPROVIDER=mistral\nMODEL=custom-model\nBOT_LANG=tr\nOWNER_ID=42\nTZ_OFFSET=5.5\n# more options: see .env.example\n");
        assert_eq!(app_id, None);
    }

    #[test]
    fn build_env_defaults_optional_answers() {
        let mut input = io::Cursor::new(b"\ntoken\n\nsk-test\n\n\n\n\n");
        let (content, _) = build_env(&mut input, false).unwrap();
        assert!(content.contains("MODEL=\nBOT_LANG=en\nOWNER_ID=\nTZ_OFFSET=0\n"));
    }

    #[test]
    fn build_env_keeps_explicit_free_model_and_trims_piped_secrets() {
        let answers = format!(
            "\r\n  token  \r\nopenrouter\r\n  sk-test  \r\n{FREE_OPENROUTER_MODEL}\r\n\r\n\r\n\r\n"
        );
        let (content, app_id) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert_eq!(
            content,
            format!("DISCORD_TOKEN=token\nOPENROUTER_KEY=sk-test\nMODEL={FREE_OPENROUTER_MODEL}\nBOT_LANG=en\nOWNER_ID=\nTZ_OFFSET=0\n# more options: see .env.example\n")
        );
        assert_eq!(app_id, None);
    }

    #[test]
    fn build_env_matrix_prefers_token_and_encodes_owner() {
        let answers = b"matrix\nhttps://matrix.example.org\n@bot:example.org\n\nsyt_test\n!one:example.org,!two:example.org\n\nsk-test\n\nen\n@owner:example.org\n\n";
        let (content, app_id) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert_eq!(content, "MATRIX_HOMESERVER=https://matrix.example.org\nMATRIX_USER=@bot:example.org\nMATRIX_ACCESS_TOKEN=syt_test\nMATRIX_ROOMS=!one:example.org,!two:example.org\nOPENROUTER_KEY=sk-test\nMODEL=\nBOT_LANG=en\nOWNER_KEY=matrix:user:%40owner%3Aexample.org\nTZ_OFFSET=0\n# more options: see .env.example\n");
        assert_eq!(app_id, None);
        assert!(!content.contains("DISCORD_TOKEN"));
        assert!(!content.contains("MATRIX_PASSWORD"));
        assert!(!content.contains("OWNER_ID"));
    }

    #[test]
    fn build_env_matrix_password_reasks_invalid_configuration() {
        let answers = b"matrix\n\nmatrix.example.org\nftp://example.org\nhttps://example.org\nbot\n@:example.org\n@bot:\n@bot:example.org\nunknown\npassword\n\nsecret\n#alias:example.org\n!room:\n\nmistral\nsk-test\ncustom\ntr\n\nowner\n@owner:example.org\n5.5\n";
        let (content, _) = build_env(&mut io::Cursor::new(answers), false).unwrap();
        assert!(content.contains("MATRIX_PASSWORD=secret\nMATRIX_ROOMS=\n"));
        assert!(content.contains("MISTRAL_KEY=sk-test\nPROVIDER=mistral\n"));
        assert!(content.contains("BOT_LANG=tr\nOWNER_KEY=matrix:user:%40owner%3Aexample.org\n"));
        assert!(content.contains("TZ_OFFSET=5.5\n"));
        assert!(!content.contains("MATRIX_ACCESS_TOKEN"));
    }

    #[test]
    fn matrix_secret_survives_dotenv_quoting() {
        let secret = r#"pa ss#'$word"\end"#;
        let content = format!("MATRIX_PASSWORD={}\n", dotenv_value(secret.to_string()));
        let entries = dotenvy::from_read_iter(content.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            entries,
            vec![("MATRIX_PASSWORD".to_string(), secret.to_string())]
        );
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

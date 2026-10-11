use super::*;
use base64::Engine;
use std::io::{self, BufRead, Write};

// View Channels, Send Messages, Embed Links, Attach Files, Read Message History,
// Add Reactions and Change Nickname.
const INVITE_PERMISSIONS: u64 = 67226688;

pub fn run(args: &[String]) -> Result<(), BotError> {
    let path = std::path::Path::new(".env");
    if path.exists() && !args.iter().any(|arg| arg == "--force") {
        return Err(".env already exists; rerun with --force to overwrite".into());
    }

    let (content, app_id) = build_env(&mut io::stdin().lock())?;
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    println!("Wrote .env");
    match app_id {
        Some(id) => println!("{}", invite_url(id)),
        None => {
            println!("Couldn't read the application id from the token. Copy it from Developer Portal → General Information and open:");
            println!("https://discord.com/oauth2/authorize?client_id=<APP_ID>&scope=bot+applications.commands&permissions={INVITE_PERMISSIONS}");
        }
    }
    println!("Enable \"Message Content\" and \"Server Members\" intents: Developer Portal → Bot → Privileged Gateway Intents.");
    println!("Start it: discord-bot (binary) · cargo run --release (source) · docker compose up -d (Docker)");
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

fn ask_required<R: BufRead>(input: &mut R, prompt: &str) -> Result<String, BotError> {
    loop {
        let answer = ask(input, prompt)?;
        if !answer.is_empty() {
            return Ok(answer);
        }
    }
}

// Collect answers without touching the filesystem, so the same flow is testable.
fn build_env<R: BufRead>(input: &mut R) -> Result<(String, Option<u64>), BotError> {
    let token = ask_required(
        input,
        "Discord bot token (Developer Portal → Bot → Reset Token): ",
    )?;
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
    let (key_name, key_prompt, default_model) = if provider == "mistral" {
        ("MISTRAL_KEY", "Mistral API key: ", MISTRAL_MODEL)
    } else {
        ("OPENROUTER_KEY", "OpenRouter API key: ", OPENROUTER_MODEL)
    };
    let key = ask_required(input, key_prompt)?;
    let model = ask(input, &format!("Model id (Enter = {default_model}): "))?;
    let language = loop {
        let answer = ask(input, "Bot language [en/tr] (Enter = en): ")?;
        match answer.as_str() {
            "" | "en" => break "en",
            "tr" => break "tr",
            _ => continue,
        }
    };
    let owner = loop {
        let answer = ask(
            input,
            "Your Discord user id — the owner, always liked, can switch /model (Enter = skip): ",
        )?;
        if answer.bytes().all(|byte| byte.is_ascii_digit()) {
            break answer;
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

    let mut content = format!("DISCORD_TOKEN={token}\n{key_name}={key}\n");
    if provider == "mistral" {
        content.push_str("PROVIDER=mistral\n");
    }
    content.push_str(&format!(
        "MODEL={model}\nBOT_LANG={language}\nOWNER_ID={owner}\nTZ_OFFSET={offset}\n# more options: see .env.example\n"
    ));
    Ok((content, app_id_from_token(&token)))
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
        let mut input = io::Cursor::new(b"MTIzNDU2Nzg5MDEyMzQ1Njc4.x.y\n\nsk-test\n\nen\n42\n-5\n");
        let (content, app_id) = build_env(&mut input).unwrap();
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
            b"\ntoken\nunknown\nmistral\n\nsk-test\ncustom-model\nde\ntr\ninvalid\n42\n15\n5.5\n",
        );
        let (content, app_id) = build_env(&mut input).unwrap();
        assert_eq!(content, "DISCORD_TOKEN=token\nMISTRAL_KEY=sk-test\nPROVIDER=mistral\nMODEL=custom-model\nBOT_LANG=tr\nOWNER_ID=42\nTZ_OFFSET=5.5\n# more options: see .env.example\n");
        assert_eq!(app_id, None);
    }

    #[test]
    fn build_env_defaults_optional_answers() {
        let mut input = io::Cursor::new(b"token\n\nsk-test\n\n\n\n\n");
        let (content, _) = build_env(&mut input).unwrap();
        assert!(content.contains("MODEL=\nBOT_LANG=en\nOWNER_ID=\nTZ_OFFSET=0\n"));
    }

    #[test]
    fn ask_reports_eof() {
        let error = ask(&mut io::Cursor::new(b""), "prompt: ").unwrap_err();
        assert_eq!(error.to_string(), "input ended");
    }
}

use std::collections::HashMap;
use std::sync::LazyLock;

use askama::Template;

pub type ViewError = Box<dyn std::error::Error + Send + Sync>;

pub const CSS: &str = include_str!("assets/panel.css");
pub const JS: &str = include_str!("assets/panel.js");
pub const HTMX_JS: &str = include_str!("assets/htmx-2.0.4.min.js");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    En,
    Tr,
}

impl Language {
    pub fn parse(value: &str) -> Self {
        match value {
            "tr" => Self::Tr,
            _ => Self::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Tr => "tr",
        }
    }
}

pub fn text(language: Language, key: &str) -> Result<&'static str, ViewError> {
    type Table = Result<HashMap<String, String>, serde_json::Error>;
    static EN: LazyLock<Table> =
        LazyLock::new(|| serde_json::from_str(include_str!("../../langs/en.json")));
    static TR: LazyLock<Table> =
        LazyLock::new(|| serde_json::from_str(include_str!("../../langs/tr.json")));
    let table = match language {
        Language::En => &*EN,
        Language::Tr => &*TR,
    };
    match table {
        Ok(table) => table
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing panel locale key: {key}").into()),
        Err(error) => Err(format!("invalid panel locale: {error}").into()),
    }
}

#[derive(Clone, Debug, Default)]
pub struct NavItem {
    pub title: String,
    pub href: String,
    pub active: bool,
    pub disabled: bool,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct Step {
    pub title: String,
    pub href: String,
    pub current: bool,
    pub complete: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub label: String,
    pub value: String,
    pub detail: String,
    /// Optional dotted path in the dashboard JSON snapshot; values become text nodes.
    pub key: String,
    /// Verified link supplied by the service, never persisted user text.
    pub href: String,
}

#[derive(Clone, Debug, Default)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Field {
    pub name: String,
    pub label: String,
    /// text, password, select, textarea, checkbox, number, or url.
    pub kind: String,
    pub value: String,
    pub help: String,
    pub error: String,
    pub required: bool,
    pub read_only: bool,
    pub checked: bool,
    pub autocomplete: String,
    pub choices: Vec<Choice>,
    /// Already-localized configuration state, never a secret value.
    pub configured: String,
    pub source: String,
    pub effective: String,
    pub saved: String,
    pub default: String,
}

#[derive(Clone, Debug, Default)]
pub struct Button {
    pub label: String,
    pub name: String,
    pub value: String,
    /// Optional POST endpoint for a distinct validation/test operation.
    pub action: String,
    pub secondary: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Form {
    pub id: String,
    pub action: String,
    pub csrf: String,
    pub hidden: Vec<(String, String)>,
    pub fields: Vec<Field>,
    pub buttons: Vec<Button>,
}

#[derive(Clone, Debug, Default)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub description: String,
    pub items: Vec<Item>,
    pub forms: Vec<Form>,
    pub advanced: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub language: Language,
    pub title: String,
    pub description: String,
    pub notice: String,
    pub error: String,
    pub nav: Vec<NavItem>,
    pub sections: Vec<Section>,
    pub steps: Vec<Step>,
    pub authenticated: bool,
    /// Nonempty only on the overview. This endpoint must return a JSON snapshot.
    pub poll_url: String,
    /// Nonempty only on logs. Requires an authenticated, bounded SSE endpoint.
    pub log_stream_url: String,
}

impl Page {
    pub fn new(language: Language, title_key: &str, active: &str) -> Result<Self, ViewError> {
        let mut nav = Vec::with_capacity(9);
        for (name, href, disabled) in [
            ("overview", "/", false),
            ("memory", "", true),
            ("personality", "", true),
            ("actions", "", true),
            ("usage", "", true),
            ("logs", "/logs", false),
            ("settings", "/settings", false),
            ("account", "/account", false),
            ("help", "/help", false),
        ] {
            nav.push(NavItem {
                title: text(language, &format!("panel.nav.{name}"))?.to_owned(),
                href: if href.is_empty() {
                    String::new()
                } else {
                    format!("{href}?lang={}", language.code())
                },
                active: name == active,
                disabled,
                reason: if disabled {
                    text(language, "panel.future")?.to_owned()
                } else {
                    String::new()
                },
            });
        }
        Ok(Self {
            language,
            title: text(language, title_key)?.to_owned(),
            nav,
            ..Self::default()
        })
    }

    pub fn setup_steps(&mut self, current: usize) -> Result<(), ViewError> {
        self.steps = (1..=5)
            .map(|step| {
                Ok(Step {
                    title: text(self.language, &format!("panel.setup.step{step}"))?.to_owned(),
                    // Sequential progress is enforced by the server; future steps are not links.
                    href: if step < current {
                        format!("/setup?step={step}&lang={}", self.language.code())
                    } else {
                        String::new()
                    },
                    current: step == current,
                    complete: step < current,
                })
            })
            .collect::<Result<Vec<_>, ViewError>>()?;
        Ok(())
    }
}

struct Chrome {
    brand: &'static str,
    local: &'static str,
    skip: &'static str,
    navigation: &'static str,
    language: &'static str,
    english: &'static str,
    turkish: &'static str,
    configured: &'static str,
    source: &'static str,
    effective: &'static str,
    saved: &'static str,
    default: &'static str,
    log_level: &'static str,
    log_search: &'static str,
    log_pause: &'static str,
    log_resume: &'static str,
    log_follow: &'static str,
    log_connecting: &'static str,
    log_connected: &'static str,
    log_reconnecting: &'static str,
    log_lag: &'static str,
    log_empty: &'static str,
    log_all: &'static str,
    log_debug: &'static str,
    log_info: &'static str,
    log_warn: &'static str,
    log_error: &'static str,
    log_trace: &'static str,
    refresh_error: &'static str,
    no_js: &'static str,
}

#[derive(Template)]
#[template(path = "panel/base.html")]
struct Base<'a> {
    page: &'a Page,
    chrome: Chrome,
}

pub fn render(page: &Page) -> Result<String, ViewError> {
    validate_urls(page)?;
    let t = |key| text(page.language, key);
    let chrome = Chrome {
        brand: t("panel.brand")?,
        local: t("panel.local")?,
        skip: t("panel.skip")?,
        navigation: t("panel.navigation")?,
        language: t("panel.language")?,
        english: t("panel.language.en")?,
        turkish: t("panel.language.tr")?,
        configured: t("panel.field.configured")?,
        source: t("panel.field.source")?,
        effective: t("panel.field.effective")?,
        saved: t("panel.field.saved")?,
        default: t("panel.field.default")?,
        log_level: t("panel.logs.level")?,
        log_search: t("panel.logs.search")?,
        log_pause: t("panel.logs.pause")?,
        log_resume: t("panel.logs.resume")?,
        log_follow: t("panel.logs.follow")?,
        log_connecting: t("panel.logs.connecting")?,
        log_connected: t("panel.logs.connected")?,
        log_reconnecting: t("panel.logs.reconnecting")?,
        log_lag: t("panel.logs.lag")?,
        log_empty: t("panel.logs.empty")?,
        log_all: t("panel.logs.all")?,
        log_debug: t("panel.logs.debug")?,
        log_info: t("panel.logs.info")?,
        log_warn: t("panel.logs.warn")?,
        log_error: t("panel.logs.error")?,
        log_trace: t("panel.logs.trace")?,
        refresh_error: t("panel.overview.refresh_error")?,
        no_js: t("panel.logs.no_js")?,
    };
    Ok(Base { page, chrome }.render()?)
}

fn validate_urls(page: &Page) -> Result<(), ViewError> {
    let local = |value: &str| -> Result<(), ViewError> {
        if value.is_empty()
            || (value.starts_with('/')
                && !value.starts_with("//")
                && !value.contains('\\')
                && !value.chars().any(char::is_control))
        {
            Ok(())
        } else {
            Err("panel endpoint must be root-relative".into())
        }
    };
    local(&page.poll_url)?;
    local(&page.log_stream_url)?;
    for item in &page.nav {
        local(&item.href)?;
    }
    for step in &page.steps {
        local(&step.href)?;
    }
    for section in &page.sections {
        for item in &section.items {
            if item.href.is_empty() || item.href.starts_with('/') {
                local(&item.href)?;
            } else {
                let url = reqwest::Url::parse(&item.href)?;
                if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
                {
                    return Err("panel links must use credential-free HTTPS".into());
                }
            }
        }
        for form in &section.forms {
            local(&form.action)?;
            for button in &form.buttons {
                local(&button.action)?;
            }
        }
    }
    Ok(())
}

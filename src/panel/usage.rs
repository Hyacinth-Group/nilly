use super::routes::{admin, html, item, lang, page, section, t, WebError, WebResult};
use super::views::{Item, Language};
use super::App;
use crate::Metrics;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/usage", get(view))
        .route("/api/v1/usage", get(api))
}

fn category_key(tag: &str) -> &'static str {
    match tag {
        "sohbet" => "panel.usage.category.chat",
        "isteklilik" => "panel.usage.category.willingness",
        "hedef_sec" => "panel.usage.category.target",
        "ruh_hali" => "panel.usage.category.mood",
        "profilci" => "panel.usage.category.profiler",
        "gunlukcu" => "panel.usage.category.diarist",
        "hoca" => "panel.usage.category.coach",
        "elestirmen" => "panel.usage.category.critic",
        "ozetleyici_kisi" => "panel.usage.category.summary_person",
        "ozetleyici_konu" => "panel.usage.category.summary_topic",
        "ozetleyici_olaylar" => "panel.usage.category.summary_events",
        "haber_sec" => "panel.usage.category.news_pick",
        "haber_tanit" => "panel.usage.category.news_intro",
        "gezgin_sec" | "gundem_sec" => "panel.usage.category.agenda_pick",
        "gezgin_not" | "gundem_not" => "panel.usage.category.agenda_note",
        "resimci" => "panel.usage.category.image",
        "laf" => "panel.usage.category.observation",
        "sorun" => "panel.usage.category.problem",
        "hack_giris" => "panel.usage.category.hack_enter",
        "hack_devam" => "panel.usage.category.hack_continue",
        "hack_cikis" => "panel.usage.category.hack_exit",
        "isim_sec" => "panel.usage.category.name_pick",
        "isim_duyuru" => "panel.usage.category.name_announce",
        "uyandim" => "panel.usage.category.wake_announce",
        "uyanis" => "panel.usage.category.wake_analysis",
        "uyanis_cevap" => "panel.usage.category.wake_reply",
        "yoldayim" => "panel.usage.category.travel",
        "ayriliyorum" => "panel.usage.category.leaving",
        _ => "panel.usage.category.other",
    }
}

pub(super) fn metric_snapshot(metrics: Option<&Metrics>, language: Language) -> Value {
    let mut categories = metrics.map_or_else(Vec::new, |metrics| {
        metrics.reporting.categories.iter().map(|(tag, usage)| json!({
            "id":tag,"label":t(language,category_key(tag)),
            "input_tokens":usage.input.known_total,
            "output_tokens":usage.output.known_total,
            "cached_input_tokens":usage.cache.known_total,
            "reported_calls":usage.usage_reported_calls,
            "counted_successful_calls":usage.counted_successful_calls,
            "attempted_calls":null,"reporting_coverage":null,
            "field_reporting":{"input":usage.input,"output":usage.output,"cached_input":usage.cache},
            "cost":null,"currency":null
        })).collect::<Vec<_>>()
    });
    categories.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    json!({
        "period":"this_process","period_label":t(language,"panel.usage.period"),
        "persistent_history":false,"configured_runtime":metrics.is_some(),
        "counted_calls":metrics.map(|metrics|metrics.calls),
        "counted_successful_calls":metrics.map(|metrics|metrics.reporting.totals.counted_successful_calls),
        "reported_calls":metrics.map(|metrics|metrics.reporting.totals.usage_reported_calls),
        "attempted_calls":null,"unreported_calls":null,
        "input_tokens":metrics.and_then(|metrics|metrics.reporting.totals.input.known_total),
        "output_tokens":metrics.and_then(|metrics|metrics.reporting.totals.output.known_total),
        "cached_input_tokens":metrics.and_then(|metrics|metrics.reporting.totals.cache.known_total),
        "field_reporting":metrics.map(|metrics|json!({"input":metrics.reporting.totals.input,"output":metrics.reporting.totals.output,"cached_input":metrics.reporting.totals.cache})),
        "cached_tokens_are_input_subset":true,"reporting_coverage":null,
        "last_counted_call_unix":metrics.and_then(|metrics|(metrics.last_call_secs>0).then_some(metrics.last_call_secs)),
        "cost":null,"currency":null,"unreported_cost_calls":null,
        "categories":categories,"accounting_note":t(language,"panel.usage.accounting_note"),
        "billing_dashboards":[
            {"provider":"openrouter","label":t(language,"panel.usage.billing.openrouter"),"href":"https://openrouter.ai/settings/credits"},
            {"provider":"mistral","label":t(language,"panel.usage.billing.mistral"),"href":"https://console.mistral.ai/"}
        ]
    })
}

fn snapshot(app: &App, language: Language) -> Value {
    let bot = app.runtime.lock().bot.clone();
    match bot {
        Some(bot) => metric_snapshot(Some(&bot.state().metrics), language),
        None => metric_snapshot(None, language),
    }
}

fn validate_query(query: &HashMap<String, String>) -> WebResult<()> {
    if query
        .get("period")
        .is_some_and(|period| period != "this_process")
        || query
            .get("group_by")
            .is_some_and(|group| group != "category")
    {
        return Err(WebError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "usage_period_unsupported",
            "panel.usage.process_only",
        ));
    }
    Ok(())
}

async fn api(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    admin(&app, &headers)?;
    validate_query(&query)?;
    Ok(Json(snapshot(&app, lang(&query))).into_response())
}

fn display(value: &Value, language: Language) -> String {
    value.as_u64().map_or_else(
        || t(language, "panel.state.not_reported"),
        |value| value.to_string(),
    )
}

async fn view(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> WebResult<Response> {
    admin(&app, &headers)?;
    validate_query(&query)?;
    let language = lang(&query);
    let snapshot = snapshot(&app, language);
    let mut result = page(language, "panel.title.usage", "usage")?;
    result.authenticated = true;
    result.description = t(language, "panel.usage.description");
    if !snapshot["configured_runtime"].as_bool().unwrap_or(false) {
        result.notice = t(language, "panel.usage.unconfigured");
        let mut welcome = section(language, "panel.overview.finish_setup");
        welcome.description = t(language, "panel.usage.description");
        welcome.items.push(Item {
            value: t(language, "panel.action.finish_setup"),
            href: format!("/setup?lang={}", language.code()),
            ..Default::default()
        });
        result.sections.push(welcome);
        return html(&result);
    }
    let mut totals = section(language, "panel.usage.period");
    totals.description = t(language, "panel.usage.cache_note");
    for (field, label) in [
        ("input_tokens", "panel.usage.input"),
        ("output_tokens", "panel.usage.output"),
        ("cached_input_tokens", "panel.usage.cached"),
        ("counted_calls", "panel.usage.counted_calls"),
    ] {
        totals.items.push(item(
            language,
            label,
            display(&snapshot[field], language),
            "",
        ));
    }
    result.sections.push(totals);
    let mut coverage = section(language, "panel.usage.coverage");
    coverage.description = t(language, "panel.usage.accounting_note");
    for (field, label) in [
        ("counted_successful_calls", "panel.usage.successful_calls"),
        ("reported_calls", "panel.usage.reported_calls"),
        ("attempted_calls", "panel.usage.attempted_calls"),
        ("reporting_coverage", "panel.usage.coverage"),
    ] {
        coverage.items.push(item(
            language,
            label,
            display(&snapshot[field], language),
            "",
        ));
    }
    for (field, label) in [
        ("input", "panel.usage.input"),
        ("output", "panel.usage.output"),
        ("cached_input", "panel.usage.cached"),
    ] {
        let mut counts = item(
            language,
            label,
            format!(
                "{}: {} · {}: {}",
                t(language, "panel.usage.field_reported"),
                display(
                    &snapshot["field_reporting"][field]["reported_calls"],
                    language
                ),
                t(language, "panel.usage.field_unknown"),
                display(
                    &snapshot["field_reporting"][field]["unknown_calls"],
                    language
                )
            ),
            "",
        );
        counts.detail = t(language, "panel.usage.field_reporting_note");
        coverage.items.push(counts);
    }
    result.sections.push(coverage);
    let mut categories = section(language, "panel.usage.categories");
    categories.description = t(language, "panel.usage.background_note");
    if let Some(rows) = snapshot["categories"].as_array() {
        for row in rows {
            categories.items.push(Item {
                label: row["label"].as_str().unwrap_or("").to_owned(),
                value: format!(
                    "{}: {} · {}: {} · {}: {}",
                    t(language, "panel.usage.input"),
                    display(&row["input_tokens"], language),
                    t(language, "panel.usage.output"),
                    display(&row["output_tokens"], language),
                    t(language, "panel.usage.cached"),
                    display(&row["cached_input_tokens"], language)
                ),
                detail: format!(
                    "{} · {}: {} · {}: {}",
                    row["id"].as_str().unwrap_or(""),
                    t(language, "panel.usage.reported_calls"),
                    display(&row["reported_calls"], language),
                    t(language, "panel.usage.successful_calls"),
                    display(&row["counted_successful_calls"], language)
                ),
                ..Default::default()
            });
        }
        if rows.is_empty() {
            categories
                .description
                .push_str(&format!("\n{}", t(language, "panel.usage.no_categories")));
        }
    }
    result.sections.push(categories);
    let mut cost = section(language, "panel.usage.cost");
    cost.description = t(language, "panel.usage.cost_note");
    for label in [
        "panel.usage.cost",
        "panel.usage.currency",
        "panel.usage.unreported_cost_calls",
    ] {
        cost.items.push(item(
            language,
            label,
            t(language, "panel.state.not_reported"),
            "",
        ));
    }
    for dashboard in snapshot["billing_dashboards"]
        .as_array()
        .into_iter()
        .flatten()
    {
        cost.items.push(Item {
            label: dashboard["label"].as_str().unwrap_or("").to_owned(),
            value: dashboard["label"].as_str().unwrap_or("").to_owned(),
            href: dashboard["href"].as_str().unwrap_or("").to_owned(),
            ..Default::default()
        });
    }
    result.sections.push(cost);
    html(&result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_usage_is_not_fabricated_zero() {
        let snapshot = metric_snapshot(None, Language::En);
        assert_eq!(snapshot["configured_runtime"], false);
        assert_eq!(snapshot["period"], "this_process");
        assert_eq!(snapshot["persistent_history"], false);
        for field in [
            "input_tokens",
            "output_tokens",
            "cached_input_tokens",
            "counted_calls",
            "reported_calls",
            "attempted_calls",
            "reporting_coverage",
            "cost",
            "currency",
        ] {
            assert!(snapshot[field].is_null(), "{field}");
        }
    }

    #[test]
    fn cache_is_subset_and_absent_cost_and_coverage_stay_unknown() {
        let mut metrics = Metrics {
            calls: 3,
            input_tokens: 100,
            output_tokens: 20,
            cache_tokens: 40,
            ..Default::default()
        };
        metrics.reporting.totals.counted_successful_calls = 3;
        metrics.reporting.totals.usage_reported_calls = 2;
        metrics.reporting.totals.input.known_total = Some(100);
        metrics.reporting.totals.output.known_total = Some(20);
        metrics.reporting.totals.cache.known_total = Some(40);
        let snapshot = metric_snapshot(Some(&metrics), Language::En);
        assert_eq!(snapshot["input_tokens"], 100);
        assert_eq!(snapshot["output_tokens"], 20);
        assert_eq!(snapshot["cached_input_tokens"], 40);
        assert_eq!(snapshot["counted_calls"], 3);
        assert_eq!(snapshot["cached_tokens_are_input_subset"], true);
        assert_eq!(snapshot["reported_calls"], 2);
        for field in ["attempted_calls", "cost", "currency", "reporting_coverage"] {
            assert!(snapshot[field].is_null(), "{field}");
        }
        let missing = metric_snapshot(Some(&Metrics::default()), Language::En);
        assert!(missing["cached_input_tokens"].is_null());
        assert!(missing["input_tokens"].is_null());
        assert_eq!(missing["counted_calls"], 0); // An observed bookkeeping counter, not provider usage.
        let mut explicit_zero = Metrics::default();
        explicit_zero.reporting.totals.cache.known_total = Some(0);
        explicit_zero.reporting.totals.input.known_total = Some(0);
        let zero = metric_snapshot(Some(&explicit_zero), Language::En);
        assert_eq!(zero["cached_input_tokens"], 0);
        assert_eq!(zero["input_tokens"], 0);
    }

    #[test]
    fn category_labels_are_localized_without_changing_metric_ids() {
        let mut metrics = Metrics::default();
        metrics
            .reporting
            .categories
            .insert("hoca", Default::default());
        for (language, expected) in [
            (Language::En, "Temperament coach"),
            (Language::Tr, "Mizaç koçu"),
        ] {
            let snapshot = metric_snapshot(Some(&metrics), language);
            assert_eq!(snapshot["categories"][0]["id"], "hoca");
            assert_eq!(snapshot["categories"][0]["label"], expected);
            assert!(snapshot["categories"][0]["cached_input_tokens"].is_null());
            assert_eq!(snapshot["categories"][0]["reported_calls"], 0);
        }
        for tag in [
            "sohbet",
            "isteklilik",
            "hedef_sec",
            "ruh_hali",
            "profilci",
            "gunlukcu",
            "hoca",
            "elestirmen",
            "ozetleyici_kisi",
            "ozetleyici_konu",
            "ozetleyici_olaylar",
            "haber_sec",
            "haber_tanit",
            "gezgin_sec",
            "gezgin_not",
            "resimci",
            "laf",
            "sorun",
            "hack_giris",
            "hack_devam",
            "hack_cikis",
            "isim_sec",
            "isim_duyuru",
            "uyandim",
            "uyanis",
            "uyanis_cevap",
            "yoldayim",
            "ayriliyorum",
            "future_category",
        ] {
            for language in [Language::En, Language::Tr] {
                let key = category_key(tag);
                assert!(
                    super::super::views::text(language, key).is_ok(),
                    "{} {key}",
                    language.code()
                );
            }
        }
    }

    #[test]
    fn history_requests_are_rejected_instead_of_relabelled() {
        assert!(validate_query(&HashMap::from([("period".into(), "daily".into())])).is_err());
        assert!(validate_query(&HashMap::from([("group_by".into(), "model".into())])).is_err());
    }
}

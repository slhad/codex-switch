use crate::cli::ProfileName;
use crate::data::{Context, ModelUsageDayTotal, ModelUsageResponse, ModelUsageTotal};
use crate::waybar::{collect_profile_usage_with_model_usage, update_last_quota_hit, ProfileUsage};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageReport {
    generated_at: String,
    profiles: Vec<ProfileUsageReport>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileUsageReport {
    profile: String,
    current: bool,
    status: String,
    days: usize,
    units: Option<String>,
    group_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fetched_at: Option<String>,
    models: Vec<ModelUsageReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    daily: Vec<ModelUsageDayReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ModelUsageReport {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<String>,
    credits: f64,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ModelUsageDayReport {
    date: String,
    models: Vec<ModelUsageReport>,
}

#[derive(Debug)]
struct ProfileBuilder {
    key: String,
    profile: String,
    current: bool,
    status: String,
    fetched_at: Option<String>,
    model_usage: Option<ModelUsageResponse>,
    error: Option<String>,
}

pub fn print_usage(ctx: &Context, profile: Option<&ProfileName>, json: bool) {
    let filter = profile.map(|profile| profile.as_str());
    let entries = collect_profile_usage_with_model_usage(ctx);
    let _ = update_last_quota_hit(ctx, &entries);
    let report = build_report(&entries, filter);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{\"profiles\":[]}".into())
        );
    } else {
        print_human_report(&report);
    }
}

fn build_report(entries: &[ProfileUsage], filter: Option<&str>) -> UsageReport {
    let mut builders = Vec::new();

    for entry in entries {
        if filter.is_some_and(|filter| entry.name != filter) {
            continue;
        }

        let key = account_key(entry);
        let index = builders
            .iter()
            .position(|builder: &ProfileBuilder| builder.key == key);
        let index = match index {
            Some(index) => index,
            None => {
                builders.push(ProfileBuilder {
                    key,
                    profile: entry.name.clone(),
                    current: entry.is_live,
                    status: "unavailable".to_string(),
                    fetched_at: None,
                    model_usage: None,
                    error: None,
                });
                builders.len() - 1
            }
        };

        let builder = &mut builders[index];
        builder.current |= entry.is_live;
        if is_placeholder(&builder.profile) && !is_placeholder(&entry.name) {
            builder.profile = entry.name.clone();
        }

        match &entry.usage {
            Ok(usage) => {
                if usage.cached {
                    if builder.status == "unavailable" {
                        builder.status = "stale".to_string();
                    }
                } else {
                    builder.status = "ok".to_string();
                }
                if entry.is_live || builder.fetched_at.is_none() {
                    builder.fetched_at = usage.last_fetched_at.clone();
                }
                if builder.error.is_none() {
                    builder.error = usage.cache_error.clone();
                }
                if let Some(model_usage) = usage.model_usage.as_ref() {
                    let has_models = !model_usage.totals().is_empty();
                    let should_replace = match builder.model_usage.as_ref() {
                        None => true,
                        Some(current) => current.totals().is_empty() && has_models,
                    };
                    if should_replace {
                        builder.model_usage = Some(model_usage.clone());
                    }
                }
            }
            Err(error) => {
                if builder.error.is_none() {
                    builder.error = Some(error.clone());
                }
            }
        }
    }

    UsageReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        profiles: builders.into_iter().map(profile_report).collect(),
    }
}

fn profile_report(builder: ProfileBuilder) -> ProfileUsageReport {
    let (days, units, group_by, models, daily) = builder
        .model_usage
        .as_ref()
        .map(|usage| {
            (
                usage.day_count(),
                usage.units.clone(),
                usage.group_by.clone(),
                usage.totals().into_iter().map(model_report).collect(),
                usage.daily_totals().iter().map(day_report).collect(),
            )
        })
        .unwrap_or((0, None, None, Vec::new(), Vec::new()));

    ProfileUsageReport {
        profile: builder.profile,
        current: builder.current,
        status: builder.status,
        days,
        units,
        group_by,
        fetched_at: builder.fetched_at,
        models,
        daily,
        error: builder.error,
    }
}

fn model_report(total: ModelUsageTotal) -> ModelUsageReport {
    ModelUsageReport {
        model: total.model,
        speed: total.speed,
        credits: total.credits,
    }
}

fn day_report(day: &ModelUsageDayTotal) -> ModelUsageDayReport {
    ModelUsageDayReport {
        date: day.date.clone(),
        models: day.models.iter().cloned().map(model_report).collect(),
    }
}

fn account_key(entry: &ProfileUsage) -> String {
    if let Some(account_id) = entry.account_id.as_deref().filter(|id| !id.is_empty()) {
        return format!("account:{}", account_id);
    }

    if !is_placeholder(&entry.email) {
        return format!("email:{}", entry.email.to_ascii_lowercase());
    }

    format!("source:{}:{}", entry.provider, entry.name)
}

fn is_placeholder(value: &str) -> bool {
    value.is_empty() || value == "?" || value == "live" || value == "unknown"
}

fn print_human_report(report: &UsageReport) {
    if report.profiles.is_empty() {
        println!("No OAuth profiles found.");
        return;
    }

    println!("Model usage by profile:");
    for profile in &report.profiles {
        let current = if profile.current { " · current" } else { "" };
        let freshness = profile
            .fetched_at
            .as_deref()
            .map(|fetched_at| format!(" · fetched {}", fetched_at))
            .unwrap_or_default();
        println!(
            "{}{} · {}{}",
            profile.profile, current, profile.status, freshness
        );

        if profile.models.is_empty() {
            let message = profile
                .error
                .as_deref()
                .unwrap_or("no model usage returned");
            println!("  {}", message);
            continue;
        }

        let period = if profile.days == 0 {
            "reported period".to_string()
        } else {
            format!("last {} days", profile.days)
        };
        let units = match profile.units.as_deref() {
            Some(units) if units.eq_ignore_ascii_case("percent") => "percentage points",
            Some(units) => units,
            None => "reported units",
        };
        println!("  {} · {}", period, units);
        for model in &profile.models {
            let speed = model
                .speed
                .as_deref()
                .map(|speed| format!(" ({})", speed))
                .unwrap_or_default();
            println!(
                "    {}{}: {}",
                model.model,
                speed,
                format_usage_value(model.credits, profile.units.as_deref())
            );
        }
    }
}

fn format_usage_value(value: f64, units: Option<&str>) -> String {
    let mut formatted = format!("{value:.2}");
    while formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    if units.is_some_and(|units| units.eq_ignore_ascii_case("percent")) {
        formatted.push_str(" pp");
    }
    formatted
}

#[cfg(test)]
mod tests {
    use super::{build_report, format_usage_value, ModelUsageReport};
    use crate::data::UsageResponse;
    use crate::waybar::ProfileUsage;

    fn entry(
        provider: &'static str,
        profile: &str,
        email: &str,
        account_id: Option<&str>,
        live: bool,
        usage: Result<UsageResponse, &str>,
    ) -> ProfileUsage {
        ProfileUsage {
            provider,
            session_id: format!("{}:{}", provider, profile),
            name: profile.to_string(),
            email: email.to_string(),
            account_id: account_id.map(str::to_string),
            is_live: live,
            usage: usage.map_err(str::to_string),
        }
    }

    fn usage_with_models() -> UsageResponse {
        serde_json::from_value(serde_json::json!({
            "model_usage": {
                "units": "percent",
                "group_by": "day",
                "data": [{"date":"2026-09-04","models":[
                    {"model":"gpt-5.6-sol","speed":"standard","credits":12.345},
                    {"model":"gpt-5.6-luna","speed":"fast","credits":2}
                ]}]
            }
        }))
        .unwrap()
    }

    #[test]
    fn groups_sources_and_keeps_model_totals() {
        let entries = vec![
            entry(
                "codex",
                "work",
                "person@example.com",
                Some("acct-work"),
                true,
                Ok(usage_with_models()),
            ),
            entry(
                "pi",
                "work-pi",
                "person@example.com",
                Some("acct-work"),
                false,
                Err("temporary failure"),
            ),
            entry(
                "codex",
                "offline",
                "other@example.com",
                Some("acct-other"),
                false,
                Err("unavailable"),
            ),
        ];

        let report = build_report(&entries, None);
        assert_eq!(report.profiles.len(), 2);
        assert_eq!(report.profiles[0].profile, "work");
        assert!(report.profiles[0].current);
        assert_eq!(report.profiles[0].status, "ok");
        assert_eq!(report.profiles[0].days, 1);
        assert_eq!(report.profiles[0].models.len(), 2);
        assert_eq!(report.profiles[0].models[0].model, "gpt-5.6-sol");
        assert_eq!(report.profiles[0].models[0].credits, 12.345);
        assert_eq!(report.profiles[0].daily.len(), 1);
        assert_eq!(report.profiles[0].daily[0].date, "2026-09-04");
        assert_eq!(report.profiles[0].daily[0].models.len(), 2);
        assert_eq!(report.profiles[1].status, "unavailable");
        assert_eq!(report.profiles[1].error.as_deref(), Some("unavailable"));
    }

    #[test]
    fn filters_profiles_and_replaces_placeholder_name() {
        let entries = vec![
            entry(
                "codex",
                "live",
                "person@example.com",
                Some("acct-work"),
                true,
                Err("failed"),
            ),
            entry(
                "pi",
                "work",
                "person@example.com",
                Some("acct-work"),
                false,
                Ok(usage_with_models()),
            ),
        ];

        let report = build_report(&entries, Some("work"));
        assert_eq!(report.profiles.len(), 1);
        assert_eq!(report.profiles[0].profile, "work");
        assert!(!report.profiles[0].current);
        assert_eq!(report.profiles[0].status, "ok");
    }

    #[test]
    fn formats_units_without_unnecessary_zeroes() {
        assert_eq!(format_usage_value(74.0, Some("percent")), "74 pp");
        assert_eq!(format_usage_value(12.345, Some("percent")), "12.35 pp");
        assert_eq!(format_usage_value(12.0, Some("credits")), "12");
        assert_eq!(format_usage_value(0.5, None), "0.5");
    }

    #[test]
    fn serializes_model_rows_with_optional_speed() {
        let row = ModelUsageReport {
            model: "other".to_string(),
            speed: None,
            credits: 1.0,
        };
        let json = serde_json::to_value(row).unwrap();
        assert_eq!(json["model"], "other");
        assert_eq!(json["credits"], 1.0);
        assert!(json.get("speed").is_none());

        let empty: UsageResponse = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(empty.model_usage.is_none());
    }
}

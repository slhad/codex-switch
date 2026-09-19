use crate::data::{
    Context, CreditAmount, ModelUsageDayTotal, ModelUsageResponse, ResetAt, TokenUsageResponse,
    TrackedQuotaHit, UsageResponse, UsageWindow,
};
use crate::rate_limit::parse_reset_at;
use crate::waybar::{
    collect_profile_usage_with_model_and_token_usage, update_last_quota_hit, ProfileUsage,
};
use chrono::{TimeZone, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageSnapshot {
    schema_version: u8,
    generated_at: String,
    accounts: Vec<AccountSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_quota_hit: Option<LastQuotaHit>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountSnapshot {
    key: String,
    name: String,
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    current: bool,
    sources: Vec<AccountSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quota: Option<QuotaSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_fetched_at: Option<String>,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountSource {
    provider: String,
    profile: String,
    live: bool,
    switchable: bool,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaSnapshot {
    plan_type: Option<String>,
    windows: Vec<QuotaWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    monthly: Option<MonthlyQuota>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset_credits: Option<ResetCredits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_usage: Option<ModelUsageSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_usage: Option<TokenUsageSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelUsageSnapshot {
    units: Option<String>,
    group_by: Option<String>,
    days: usize,
    models: Vec<ModelUsageTotalSnapshot>,
    daily: Vec<ModelUsageDaySnapshot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelUsageTotalSnapshot {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<String>,
    credits: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelUsageDaySnapshot {
    date: String,
    models: Vec<ModelUsageTotalSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenUsageSnapshot {
    days: usize,
    total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_daily_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaWindow {
    kind: &'static str,
    used_percent: Option<f64>,
    remaining_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonthlyQuota {
    limit: Option<f64>,
    used: Option<f64>,
    remaining: Option<f64>,
    used_percent: Option<f64>,
    remaining_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset_at: Option<String>,
    reached: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetCredits {
    available_count: Option<u64>,
    applicable_available_count: Option<u64>,
    credits: Vec<ResetCredit>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetCredit {
    status: Option<String>,
    title: Option<String>,
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LastQuotaHit {
    provider: Option<String>,
    profile: Option<String>,
    email: Option<String>,
    window: Option<String>,
    used_percent: Option<f64>,
    observed_at: Option<String>,
}

#[derive(Debug)]
struct AccountBuilder {
    key: String,
    name: String,
    email: String,
    account_id: Option<String>,
    current: bool,
    sources: Vec<AccountSource>,
    quota: Option<QuotaSnapshot>,
    last_fetched_at: Option<String>,
    quota_stale: bool,
    errors: Vec<String>,
}

pub fn print_snapshot(ctx: &Context) {
    let entries = collect_profile_usage_with_model_and_token_usage(ctx);
    let last_quota_hit = update_last_quota_hit(ctx, &entries);
    let snapshot = build_snapshot(&entries, last_quota_hit.as_ref());
    println!(
        "{}",
        serde_json::to_string(&snapshot).unwrap_or_else(|_| "{\"accounts\":[]}".to_string())
    );
}

fn build_snapshot(
    entries: &[ProfileUsage],
    last_quota_hit: Option<&TrackedQuotaHit>,
) -> UsageSnapshot {
    let mut builders = Vec::new();

    for entry in entries {
        let key = account_key(entry);
        let index = builders
            .iter()
            .position(|builder: &AccountBuilder| builder.key == key);
        let index = match index {
            Some(index) => index,
            None => {
                builders.push(AccountBuilder {
                    key: key.clone(),
                    name: entry.name.clone(),
                    email: entry.email.clone(),
                    account_id: entry.account_id.clone(),
                    current: entry.is_live,
                    sources: Vec::new(),
                    quota: None,
                    last_fetched_at: None,
                    quota_stale: false,
                    errors: Vec::new(),
                });
                builders.len() - 1
            }
        };

        let builder = &mut builders[index];
        builder.current |= entry.is_live;
        if builder.account_id.is_none() {
            builder.account_id = entry.account_id.clone();
        }
        if is_placeholder(&builder.name) && !is_placeholder(&entry.name) {
            builder.name = entry.name.clone();
        }
        if is_placeholder(&builder.email) && !is_placeholder(&entry.email) {
            builder.email = entry.email.clone();
        }

        let source = match &entry.usage {
            Ok(usage) => {
                let previous_model_usage = builder
                    .quota
                    .as_ref()
                    .and_then(|current| current.model_usage.clone());
                let previous_token_usage = builder
                    .quota
                    .as_ref()
                    .and_then(|current| current.token_usage.clone());
                let mut candidate = quota_snapshot(usage);
                let should_replace = builder.quota.is_none()
                    || builder.quota.as_ref().is_some_and(|current| {
                        (!quota_has_data(current) && quota_has_data(&candidate))
                            || (builder.quota_stale && !usage.cached && quota_has_data(&candidate))
                    });
                if should_replace {
                    if candidate.model_usage.is_none() {
                        candidate.model_usage = previous_model_usage;
                    }
                    if candidate.token_usage.is_none() {
                        candidate.token_usage = previous_token_usage;
                    }
                    builder.quota_stale = usage.cached;
                    builder.quota = Some(candidate);
                    builder.last_fetched_at = usage.last_fetched_at.clone();
                } else {
                    if builder
                        .quota
                        .as_ref()
                        .is_some_and(|current| current.model_usage.is_none())
                        && candidate.model_usage.is_some()
                    {
                        if let Some(quota) = builder.quota.as_mut() {
                            quota.model_usage = candidate.model_usage;
                        }
                    }
                    if builder
                        .quota
                        .as_ref()
                        .is_some_and(|current| current.token_usage.is_none())
                        && candidate.token_usage.is_some()
                    {
                        if let Some(quota) = builder.quota.as_mut() {
                            quota.token_usage = candidate.token_usage;
                        }
                    }
                }
                let error = usage
                    .cache_error
                    .as_ref()
                    .map(|error| format!("cached quota; refresh failed: {}", error));
                if let Some(error) = error.as_ref() {
                    builder.errors.push(error.clone());
                }
                AccountSource {
                    provider: entry.provider.to_string(),
                    profile: entry.name.clone(),
                    live: entry.is_live,
                    switchable: !entry.is_live,
                    status: if usage.cached { "stale" } else { "ok" },
                    error,
                }
            }
            Err(error) => {
                builder.errors.push(error.clone());
                AccountSource {
                    provider: entry.provider.to_string(),
                    profile: entry.name.clone(),
                    live: entry.is_live,
                    switchable: !entry.is_live,
                    status: "unavailable",
                    error: Some(error.clone()),
                }
            }
        };
        builder.sources.push(source);
    }

    let accounts = builders
        .into_iter()
        .map(|builder| {
            let status = if builder.quota.is_some() {
                if builder.quota_stale {
                    "stale"
                } else {
                    "ok"
                }
            } else {
                "unavailable"
            };
            AccountSnapshot {
                key: builder.key,
                name: builder.name,
                email: builder.email,
                account_id: builder.account_id,
                current: builder.current,
                sources: builder.sources,
                quota: builder.quota,
                last_fetched_at: builder.last_fetched_at,
                status,
                error: builder.errors.into_iter().next(),
            }
        })
        .collect();

    UsageSnapshot {
        schema_version: 1,
        generated_at: Utc::now().to_rfc3339(),
        accounts,
        last_quota_hit: last_quota_hit.map(last_quota_hit_snapshot),
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

fn quota_has_data(quota: &QuotaSnapshot) -> bool {
    !quota.windows.is_empty()
        || quota.monthly.is_some()
        || quota.reset_credits.is_some()
        || quota.model_usage.is_some()
        || quota.token_usage.is_some()
}

fn quota_snapshot(usage: &UsageResponse) -> QuotaSnapshot {
    let mut windows = Vec::new();
    if let Some(window) = usage.five_hour_window() {
        windows.push(quota_window("5h", window));
    }
    if let Some(window) = usage.weekly_window() {
        windows.push(quota_window("7d", window));
    }

    let monthly = usage.monthly_limit().map(|monthly| MonthlyQuota {
        limit: credit_amount(monthly.limit.as_ref()),
        used: credit_amount(monthly.used.as_ref()),
        remaining: credit_amount(monthly.remaining.as_ref()),
        used_percent: monthly.used_percent,
        remaining_percent: monthly.remaining_percent,
        reset_at: reset_at_string(monthly.reset_at.as_ref()),
        reached: usage
            .spend_control
            .as_ref()
            .and_then(|control| control.reached),
    });

    let reset_credits = usage
        .rate_limit_reset_credits
        .as_ref()
        .map(|credits| ResetCredits {
            available_count: credits.available_count,
            applicable_available_count: credits.applicable_available_count,
            credits: credits
                .credits
                .iter()
                .map(|credit| ResetCredit {
                    status: credit.status.clone(),
                    title: credit.title.clone(),
                    description: credit.description.clone(),
                    expires_at: credit.expires_at.clone(),
                })
                .collect(),
        });

    let model_usage = usage.model_usage.as_ref().map(model_usage_snapshot);
    let token_usage = usage
        .token_usage
        .as_ref()
        .filter(|usage| usage.day_count() > 0)
        .map(token_usage_snapshot);

    QuotaSnapshot {
        plan_type: usage.plan_type.clone(),
        windows,
        monthly,
        reset_credits,
        model_usage,
        token_usage,
    }
}

fn model_usage_snapshot(model_usage: &ModelUsageResponse) -> ModelUsageSnapshot {
    ModelUsageSnapshot {
        units: model_usage.units.clone(),
        group_by: model_usage.group_by.clone(),
        days: model_usage.day_count(),
        models: model_usage
            .totals()
            .into_iter()
            .map(|total| ModelUsageTotalSnapshot {
                model: total.model,
                speed: total.speed,
                credits: total.credits,
            })
            .collect(),
        daily: model_usage
            .daily_totals()
            .iter()
            .map(model_usage_day_snapshot)
            .collect(),
    }
}

fn model_usage_day_snapshot(day: &ModelUsageDayTotal) -> ModelUsageDaySnapshot {
    ModelUsageDaySnapshot {
        date: day.date.clone(),
        models: day
            .models
            .iter()
            .map(|total| ModelUsageTotalSnapshot {
                model: total.model.clone(),
                speed: total.speed.clone(),
                credits: total.credits,
            })
            .collect(),
    }
}

fn token_usage_snapshot(token_usage: &TokenUsageResponse) -> TokenUsageSnapshot {
    TokenUsageSnapshot {
        days: token_usage.day_count(),
        total_tokens: token_usage.total_daily_tokens(),
        peak_daily_tokens: token_usage.peak_daily_tokens(),
    }
}

fn quota_window(kind: &'static str, window: &UsageWindow) -> QuotaWindow {
    QuotaWindow {
        kind,
        used_percent: window.used_percent,
        remaining_percent: window
            .used_percent
            .map(|value| (100.0 - value).clamp(0.0, 100.0)),
        reset_at: reset_at_string(window.reset_at.as_ref()),
    }
}

fn credit_amount(value: Option<&CreditAmount>) -> Option<f64> {
    value.and_then(CreditAmount::as_f64)
}

fn reset_at_string(value: Option<&ResetAt>) -> Option<String> {
    let timestamp = parse_reset_at(value)?;
    let timestamp = i64::try_from(timestamp).ok()?;
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|date| date.to_rfc3339())
}

fn last_quota_hit_snapshot(hit: &TrackedQuotaHit) -> LastQuotaHit {
    LastQuotaHit {
        provider: hit.provider.clone(),
        profile: hit.profile.clone(),
        email: hit.email.clone(),
        window: hit.window.clone(),
        used_percent: hit.used_percent,
        observed_at: hit.observed_at.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        account_key, build_snapshot, credit_amount, is_placeholder, print_snapshot, quota_has_data,
        quota_snapshot, reset_at_string, ModelUsageSnapshot, MonthlyQuota, QuotaSnapshot,
        ResetCredits, TokenUsageSnapshot,
    };
    use crate::data::{Context, CreditAmount, ResetAt, TrackedQuotaHit, UsageResponse};
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

    #[test]
    fn groups_codex_and_pi_sources_for_one_account() {
        let usage: UsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit": {
                "primary_window": {"used_percent": 42.0, "reset_at": 4102444800u64},
                "secondary_window": {"used_percent": 12.0, "reset_at": 4102444800u64}
            }
        }))
        .unwrap();
        let entries = vec![
            entry(
                "codex",
                "work",
                "person@example.com",
                Some("acct-work"),
                true,
                Ok(usage),
            ),
            entry(
                "pi",
                "work-pi",
                "person@example.com",
                Some("acct-work"),
                false,
                Err("unavailable"),
            ),
            entry(
                "codex",
                "personal",
                "other@example.com",
                Some("acct-other"),
                false,
                Err("unauthorized"),
            ),
        ];

        let snapshot = build_snapshot(&entries, None);
        assert_eq!(snapshot.accounts.len(), 2);
        assert_eq!(snapshot.accounts[0].sources.len(), 2);
        assert!(snapshot.accounts[0].current);
        assert_eq!(snapshot.accounts[0].status, "ok");
        assert_eq!(snapshot.accounts[0].sources[1].status, "unavailable");
        assert_eq!(snapshot.accounts[1].status, "unavailable");
    }

    #[test]
    fn groups_email_case_insensitively_without_account_id() {
        let entries = vec![
            entry(
                "codex",
                "one",
                "Person@Example.com",
                None,
                false,
                Err("failed"),
            ),
            entry("pi", "two", "person@example.com", None, true, Err("failed")),
        ];

        let snapshot = build_snapshot(&entries, None);
        assert_eq!(snapshot.accounts.len(), 1);
        assert_eq!(snapshot.accounts[0].sources.len(), 2);
        assert!(snapshot.accounts[0].current);
    }

    #[test]
    fn handles_placeholder_metadata_and_source_identity_fallback() {
        let entries = vec![
            entry(
                "codex",
                "live",
                "?",
                Some("acct-placeholder"),
                true,
                Err("first unavailable"),
            ),
            entry(
                "pi",
                "work",
                "person@example.com",
                Some("acct-placeholder"),
                false,
                Err("second unavailable"),
            ),
        ];

        let snapshot = build_snapshot(&entries, None);
        assert_eq!(snapshot.accounts.len(), 1);
        assert_eq!(snapshot.accounts[0].name, "work");
        assert_eq!(snapshot.accounts[0].email, "person@example.com");

        let fallback = entry("codex", "fallback", "?", Some(""), false, Err("failed"));
        assert_eq!(account_key(&fallback), "source:codex:fallback");
        assert!(is_placeholder(""));
        assert!(is_placeholder("?"));
        assert!(is_placeholder("live"));
        assert!(is_placeholder("unknown"));
        assert!(!is_placeholder("work"));
    }

    #[test]
    fn replaces_an_empty_successful_quota_with_later_account_data() {
        let empty: UsageResponse = serde_json::from_value(serde_json::json!({})).unwrap();
        let full: UsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit": {"primary_window": {"used_percent": 25.0, "limit_window_seconds": 18000}}
        }))
        .unwrap();
        let entries = vec![
            entry(
                "codex",
                "work",
                "person@example.com",
                Some("acct"),
                true,
                Ok(empty),
            ),
            entry(
                "pi",
                "work-pi",
                "person@example.com",
                Some("acct"),
                false,
                Ok(full),
            ),
        ];

        let snapshot = build_snapshot(&entries, None);
        assert!(!snapshot.accounts[0]
            .quota
            .as_ref()
            .unwrap()
            .windows
            .is_empty());
    }

    #[test]
    fn covers_empty_quota_fields_and_missing_window_values() {
        let empty: UsageResponse = serde_json::from_value(serde_json::json!({})).unwrap();
        let empty_quota = quota_snapshot(&empty);
        assert!(empty_quota.windows.is_empty());
        assert!(empty_quota.monthly.is_none());
        assert!(empty_quota.reset_credits.is_none());

        let missing_percent: UsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit": {"primary_window": {"limit_window_seconds": 18000}}
        }))
        .unwrap();
        let quota = quota_snapshot(&missing_percent);
        assert!(!quota.windows.is_empty());
        assert!(quota
            .windows
            .iter()
            .all(|window| window.used_percent.is_none()));
        assert!(quota
            .windows
            .iter()
            .all(|window| window.remaining_percent.is_none()));

        assert!(credit_amount(Some(&CreditAmount::Number(12.5))).is_some());
        assert!(credit_amount(Some(&CreditAmount::String("bad".to_string()))).is_none());
        assert!(credit_amount(None).is_none());
    }

    #[test]
    fn covers_quota_data_shapes_and_reset_timestamp_failures() {
        let empty = QuotaSnapshot {
            plan_type: None,
            windows: Vec::new(),
            monthly: None,
            reset_credits: None,
            model_usage: None,
            token_usage: None,
        };
        assert!(!quota_has_data(&empty));
        assert!(quota_has_data(&QuotaSnapshot {
            windows: vec![super::QuotaWindow {
                kind: "5h",
                used_percent: None,
                remaining_percent: None,
                reset_at: None,
            }],
            ..empty_snapshot()
        }));
        assert!(quota_has_data(&QuotaSnapshot {
            monthly: Some(MonthlyQuota {
                limit: None,
                used: None,
                remaining: None,
                used_percent: None,
                remaining_percent: None,
                reset_at: None,
                reached: None,
            }),
            ..empty_snapshot()
        }));
        assert!(quota_has_data(&QuotaSnapshot {
            reset_credits: Some(ResetCredits {
                available_count: None,
                applicable_available_count: None,
                credits: Vec::new(),
            }),
            ..empty_snapshot()
        }));
        assert!(quota_has_data(&QuotaSnapshot {
            model_usage: Some(ModelUsageSnapshot {
                units: Some("percent".to_string()),
                group_by: Some("day".to_string()),
                days: 1,
                models: Vec::new(),
                daily: Vec::new(),
            }),
            ..empty_snapshot()
        }));
        assert!(quota_has_data(&QuotaSnapshot {
            token_usage: Some(TokenUsageSnapshot {
                days: 1,
                total_tokens: 100,
                peak_daily_tokens: Some(100),
            }),
            ..empty_snapshot()
        }));

        assert!(reset_at_string(None).is_none());
        assert!(reset_at_string(Some(&ResetAt::Rfc3339("bad".to_string()))).is_none());
        assert!(reset_at_string(Some(&ResetAt::Epoch(u64::MAX))).is_none());
        assert!(reset_at_string(Some(&ResetAt::Epoch(4102444800))).is_some());
    }

    fn empty_snapshot() -> QuotaSnapshot {
        QuotaSnapshot {
            plan_type: None,
            windows: Vec::new(),
            monthly: None,
            reset_credits: None,
            model_usage: None,
            token_usage: None,
        }
    }

    #[test]
    fn print_snapshot_handles_a_home_without_auth_files() {
        let base =
            std::env::temp_dir().join(format!("codex-switch-omarchy-print-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let state_dir = base.join("state");
        let ctx = Context {
            live_auth: base.join("codex").join("auth.json"),
            pi_auth: base.join("pi").join("auth.json"),
            tracker_file: state_dir.join("accounts.json"),
            state_dir,
        };

        print_snapshot(&ctx);
        assert!(ctx.tracker_file.exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn serializes_windows_monthly_limits_and_reset_credits() {
        let usage: UsageResponse = serde_json::from_value(serde_json::json!({
            "plan_type": "business",
            "rate_limit": {
                "primary_window": {"used_percent": 20.0, "limit_window_seconds": 18000, "reset_at": 4102444800u64},
                "secondary_window": {"used_percent": 35.0, "limit_window_seconds": 604800, "reset_at": 4102444800u64}
            },
            "spend_control": {"reached": false, "individual_limit": {
                "limit": "12500", "used": "100", "remaining": "12400",
                "used_percent": 1.0, "remaining_percent": 99.0, "reset_at": 4102444800u64
            }},
            "rate_limit_reset_credits": {"available_count": 1, "applicable_available_count": 0,
                "credits": [{"status": "available", "title": "Full reset", "expires_at": "2100-01-01T00:00:00Z"}]},
            "model_usage": {"units": "percent", "group_by": "day", "data": [{"date": "2026-09-04", "models": [
                {"model": "gpt-5.6-sol", "speed": "standard", "credits": 12.5}
            ]}]},
            "token_usage": {"summary": {"peakDailyTokens": 700}, "dailyUsageBuckets": [
                {"startDate": "2026-09-04", "tokens": 1200}
            ]}
        }))
        .unwrap();

        let quota = quota_snapshot(&usage);
        assert_eq!(quota.plan_type.as_deref(), Some("business"));
        assert_eq!(quota.windows.len(), 2);
        assert_eq!(quota.windows[0].kind, "5h");
        assert_eq!(quota.windows[0].remaining_percent, Some(80.0));
        assert_eq!(quota.windows[1].kind, "7d");
        assert_eq!(quota.windows[1].used_percent, Some(35.0));
        assert_eq!(quota.windows[1].remaining_percent, Some(65.0));
        assert_eq!(quota.monthly.as_ref().unwrap().used, Some(100.0));
        assert_eq!(
            quota.reset_credits.as_ref().unwrap().available_count,
            Some(1)
        );
        assert_eq!(quota.reset_credits.as_ref().unwrap().credits.len(), 1);
        let model_usage = quota.model_usage.as_ref().unwrap();
        assert_eq!(model_usage.units.as_deref(), Some("percent"));
        assert_eq!(model_usage.days, 1);
        assert_eq!(model_usage.models.len(), 1);
        assert_eq!(model_usage.models[0].model, "gpt-5.6-sol");
        assert_eq!(model_usage.models[0].credits, 12.5);
        assert_eq!(model_usage.daily.len(), 1);
        assert_eq!(model_usage.daily[0].date, "2026-09-04");
        assert_eq!(model_usage.daily[0].models[0].credits, 12.5);
        let token_usage = quota.token_usage.as_ref().unwrap();
        assert_eq!(token_usage.days, 1);
        assert_eq!(token_usage.total_tokens, 1200);
        assert_eq!(token_usage.peak_daily_tokens, Some(700));
    }

    #[test]
    fn merges_model_and_token_usage_across_account_sources() {
        let model_only: UsageResponse = serde_json::from_value(serde_json::json!({
            "model_usage": {"units": "percent", "data": [{"date": "2026-09-04", "models": [
                {"model": "gpt-5.6-sol", "credits": 12.5}
            ]}]}
        }))
        .unwrap();
        let token_only: UsageResponse = serde_json::from_value(serde_json::json!({
            "token_usage": {"dailyUsageBuckets": [
                {"startDate": "2026-09-04", "tokens": 1200}
            ]}
        }))
        .unwrap();

        let snapshot = build_snapshot(
            &[
                entry(
                    "codex",
                    "work",
                    "person@example.com",
                    Some("acct-work"),
                    true,
                    Ok(model_only),
                ),
                entry(
                    "pi",
                    "work-pi",
                    "person@example.com",
                    Some("acct-work"),
                    false,
                    Ok(token_only),
                ),
            ],
            None,
        );

        let quota = snapshot.accounts[0].quota.as_ref().unwrap();
        assert!(quota.model_usage.is_some());
        assert_eq!(quota.token_usage.as_ref().unwrap().total_tokens, 1200);
    }

    #[test]
    fn preserves_cached_quota_in_snapshot_and_marks_refresh_failure() {
        let mut usage: UsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit": {
                "primary_window": {"used_percent": 42.0, "reset_at": 4102444800u64}
            }
        }))
        .unwrap();
        usage.cached = true;
        usage.cache_error = Some("network unavailable".to_string());
        usage.last_fetched_at = Some("2026-09-02T10:00:00Z".to_string());

        let snapshot = build_snapshot(
            &[entry(
                "codex",
                "work",
                "person@example.com",
                Some("acct-work"),
                true,
                Ok(usage),
            )],
            None,
        );

        assert_eq!(snapshot.accounts[0].status, "stale");
        assert_eq!(snapshot.accounts[0].sources[0].status, "stale");
        assert_eq!(
            snapshot.accounts[0].sources[0].error.as_deref(),
            Some("cached quota; refresh failed: network unavailable")
        );
        assert_eq!(
            snapshot.accounts[0].quota.as_ref().unwrap().windows[0].used_percent,
            Some(42.0)
        );
        assert_eq!(
            snapshot.accounts[0].last_fetched_at.as_deref(),
            Some("2026-09-02T10:00:00Z")
        );
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json["accounts"][0]["lastFetchedAt"], "2026-09-02T10:00:00Z");
    }

    #[test]
    fn only_replaces_cached_quota_with_fresh_account_data() {
        let full: UsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit": {"primary_window": {"used_percent": 25.0, "reset_at": 4102444800u64}}
        }))
        .unwrap();
        let empty: UsageResponse = serde_json::from_value(serde_json::json!({})).unwrap();

        let unchanged = build_snapshot(
            &[
                entry(
                    "codex",
                    "work",
                    "person@example.com",
                    Some("acct"),
                    true,
                    Ok(full.clone()),
                ),
                entry(
                    "pi",
                    "work-pi",
                    "person@example.com",
                    Some("acct"),
                    false,
                    Ok(empty.clone()),
                ),
            ],
            None,
        );
        assert_eq!(
            unchanged.accounts[0].quota.as_ref().unwrap().windows[0].used_percent,
            Some(25.0)
        );

        let empty_only = build_snapshot(
            &[
                entry(
                    "codex",
                    "work",
                    "person@example.com",
                    Some("acct-empty"),
                    true,
                    Ok(empty.clone()),
                ),
                entry(
                    "pi",
                    "work-pi",
                    "person@example.com",
                    Some("acct-empty"),
                    false,
                    Ok(empty.clone()),
                ),
            ],
            None,
        );
        assert!(empty_only.accounts[0]
            .quota
            .as_ref()
            .unwrap()
            .windows
            .is_empty());

        let mut cached = full.clone();
        cached.cached = true;
        let refreshed = build_snapshot(
            &[
                entry(
                    "codex",
                    "work",
                    "person@example.com",
                    Some("acct-refresh"),
                    true,
                    Ok(cached.clone()),
                ),
                entry(
                    "pi",
                    "work-pi",
                    "person@example.com",
                    Some("acct-refresh"),
                    false,
                    Ok(full),
                ),
            ],
            None,
        );
        assert_eq!(refreshed.accounts[0].status, "ok");

        let stale = build_snapshot(
            &[
                entry(
                    "codex",
                    "work",
                    "person@example.com",
                    Some("acct-stale"),
                    true,
                    Ok(cached),
                ),
                entry(
                    "pi",
                    "work-pi",
                    "person@example.com",
                    Some("acct-stale"),
                    false,
                    Ok(empty),
                ),
            ],
            None,
        );
        assert_eq!(stale.accounts[0].status, "stale");

        let model_only: UsageResponse = serde_json::from_value(serde_json::json!({
            "model_usage": {
                "units": "percent",
                "data": [{"models":[{"model":"gpt-5.6-sol","credits":2.0}]}]
            }
        }))
        .unwrap();
        let model_merged = build_snapshot(
            &[
                entry(
                    "codex",
                    "work",
                    "person@example.com",
                    Some("acct-model"),
                    true,
                    Ok(serde_json::from_value(serde_json::json!({
                        "rate_limit": {"primary_window": {"used_percent": 25.0}}
                    }))
                    .unwrap()),
                ),
                entry(
                    "pi",
                    "work-pi",
                    "person@example.com",
                    Some("acct-model"),
                    false,
                    Ok(model_only),
                ),
            ],
            None,
        );
        assert_eq!(
            model_merged.accounts[0]
                .quota
                .as_ref()
                .unwrap()
                .model_usage
                .as_ref()
                .unwrap()
                .models[0]
                .model,
            "gpt-5.6-sol"
        );
    }

    #[test]
    fn includes_last_quota_hit_without_account_secrets() {
        let hit = TrackedQuotaHit {
            provider: Some("codex".to_string()),
            profile: Some("work".to_string()),
            email: Some("person@example.com".to_string()),
            window: Some("5h".to_string()),
            used_percent: Some(90.0),
            observed_at: Some("2026-01-01T00:00:00Z".to_string()),
            ..TrackedQuotaHit::default()
        };
        let snapshot = build_snapshot(&[], Some(&hit));
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json["lastQuotaHit"]["profile"], "work");
        assert!(json["lastQuotaHit"].get("accountId").is_none());
    }
}

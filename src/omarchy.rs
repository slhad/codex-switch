use crate::data::{Context, CreditAmount, ResetAt, TrackedQuotaHit, UsageResponse, UsageWindow};
use crate::rate_limit::parse_reset_at;
use crate::waybar::{collect_profile_usage, update_last_quota_hit, ProfileUsage};
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
    errors: Vec<String>,
}

pub fn print_snapshot(ctx: &Context) {
    let entries = collect_profile_usage(ctx);
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
                let candidate = quota_snapshot(usage);
                let should_replace = builder.quota.is_none()
                    || builder.quota.as_ref().is_some_and(|current| {
                        !quota_has_data(current) && quota_has_data(&candidate)
                    });
                if should_replace {
                    builder.quota = Some(candidate);
                }
                AccountSource {
                    provider: entry.provider.to_string(),
                    profile: entry.name.clone(),
                    live: entry.is_live,
                    switchable: !entry.is_live,
                    status: "ok",
                    error: None,
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
                "ok"
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
    !quota.windows.is_empty() || quota.monthly.is_some() || quota.reset_credits.is_some()
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

    QuotaSnapshot {
        plan_type: usage.plan_type.clone(),
        windows,
        monthly,
        reset_credits,
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
        quota_snapshot, reset_at_string, MonthlyQuota, QuotaSnapshot, ResetCredits,
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
            "rate_limit": {"primary_window": {"used_percent": 20.0, "limit_window_seconds": 18000, "reset_at": 4102444800u64}},
            "spend_control": {"reached": false, "individual_limit": {
                "limit": "12500", "used": "100", "remaining": "12400",
                "used_percent": 1.0, "remaining_percent": 99.0, "reset_at": 4102444800u64
            }},
            "rate_limit_reset_credits": {"available_count": 1, "applicable_available_count": 0,
                "credits": [{"status": "available", "title": "Full reset", "expires_at": "2100-01-01T00:00:00Z"}]}
        }))
        .unwrap();

        let quota = quota_snapshot(&usage);
        assert_eq!(quota.plan_type.as_deref(), Some("business"));
        assert_eq!(quota.windows[0].kind, "5h");
        assert_eq!(quota.windows[0].remaining_percent, Some(80.0));
        assert_eq!(quota.monthly.as_ref().unwrap().used, Some(100.0));
        assert_eq!(
            quota.reset_credits.as_ref().unwrap().available_count,
            Some(1)
        );
        assert_eq!(quota.reset_credits.as_ref().unwrap().credits.len(), 1);
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

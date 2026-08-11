use crate::data::{
    read_auth, read_pi_auth, Context, PiOpenAiCodexAuth, TrackedQuotaHit, UsageResponse,
};
use crate::jwt::{extract_email, extract_email_from_token};
use crate::profile::{detect_current_profile, list_pi_profiles, list_profiles, profile_name};
use crate::rate_limit::{
    fetch_pi_rate_limit, fetch_pi_rate_limit_for_path, fetch_rate_limit_for_auth_path,
    format_credit_amount, format_duration_until, parse_reset_at, summarize_reset, summarize_window,
};
use crate::tracker::{load_tracker, save_tracker, update_monthly_usage, update_rate_limit};
use chrono::{DateTime, Local, Utc};
use serde::Serialize;

const DEFAULT_FORMAT: &str = "{usage_block}";
const ICON: &str = "\u{F915}";
const TIME_ICON: &str = "󰥔";

#[derive(Debug)]
struct ProfileUsage {
    provider: &'static str,
    session_id: String,
    name: String,
    email: String,
    account_id: Option<String>,
    is_live: bool,
    usage: Result<UsageResponse, String>,
}

#[derive(Serialize)]
struct WaybarOutput {
    text: String,
    tooltip: String,
    alt: String,
    class: String,
    percentage: u8,
}

pub fn print_waybar(
    ctx: &Context,
    format: Option<&str>,
    tooltip_format: Option<&str>,
    hide_minutes_with_days: bool,
    hide_hours_with_days: bool,
    percent_left: bool,
) {
    let entries = collect_profile_usage(ctx);
    let last_quota_hit = update_last_quota_hit(ctx, &entries);
    let active = display_entry(&entries, last_quota_hit.as_ref());

    let text = active
        .map(|entry| {
            format_entry_with_options(
                format.unwrap_or(DEFAULT_FORMAT),
                entry,
                last_quota_hit.as_ref(),
                hide_minutes_with_days,
                hide_hours_with_days,
                percent_left,
            )
        })
        .unwrap_or_else(|| "codex ?".to_string());
    let percentage = active.and_then(entry_percentage).unwrap_or(0);
    let class = class_for_percentage(percentage).to_string();
    let tooltip = if let Some(template) = tooltip_format {
        active
            .map(|entry| {
                format_entry_with_options(
                    template,
                    entry,
                    last_quota_hit.as_ref(),
                    false,
                    false,
                    percent_left,
                )
            })
            .unwrap_or_else(|| "No Codex profiles found".to_string())
    } else {
        format_tooltip(&entries, last_quota_hit.as_ref())
    };
    let alt = format_alt(&entries, last_quota_hit.as_ref(), percent_left);

    let output = WaybarOutput {
        text,
        tooltip,
        alt,
        class,
        percentage,
    };
    println!(
        "{}",
        serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_string())
    );
}

fn display_entry<'a>(
    entries: &'a [ProfileUsage],
    last_hit: Option<&TrackedQuotaHit>,
) -> Option<&'a ProfileUsage> {
    last_hit
        .and_then(|hit| {
            hit.session_id
                .as_deref()
                .and_then(|session_id| entries.iter().find(|entry| entry.session_id == session_id))
                .or_else(|| {
                    entries.iter().find(|entry| {
                        hit.provider.as_deref() == Some(entry.provider)
                            && hit.profile.as_deref() == Some(entry.name.as_str())
                    })
                })
                .or_else(|| {
                    entries.iter().find(|entry| {
                        hit.provider.as_deref() == Some(entry.provider)
                            && hit.email.as_deref() == Some(entry.email.as_str())
                    })
                })
        })
        .or_else(|| {
            entries
                .iter()
                .find(|entry| entry.provider == "codex" && entry.is_live)
        })
        .or_else(|| entries.iter().find(|entry| entry.is_live))
        .or_else(|| entries.first())
}

fn collect_profile_usage(ctx: &Context) -> Vec<ProfileUsage> {
    let mut entries = Vec::new();
    let live_bytes = std::fs::read(&ctx.live_auth).ok();
    let live_pi = read_pi_auth(&ctx.pi_auth).and_then(|auth| auth.openai_codex);

    if ctx.live_auth.exists() {
        let current_profile = detect_current_profile(ctx).unwrap_or_else(|| "live".to_string());
        let usage = fetch_rate_limit_for_auth_path(&ctx.live_auth).map(|(usage, _)| usage);
        let email = std::panic::catch_unwind(|| read_auth(&ctx.live_auth))
            .ok()
            .and_then(|auth| extract_email(&auth))
            .unwrap_or_else(|| "?".to_string());
        let account_id = std::panic::catch_unwind(|| read_auth(&ctx.live_auth))
            .ok()
            .and_then(|auth| auth.tokens.account_id);
        entries.push(ProfileUsage {
            provider: "codex",
            session_id: "codex:live".to_string(),
            name: current_profile,
            email,
            account_id,
            is_live: true,
            usage,
        });
    }

    for path in list_profiles(ctx) {
        if Some(std::fs::read(&path).unwrap_or_default()) == live_bytes {
            continue;
        }
        let name = profile_name(&path);
        let auth = std::panic::catch_unwind(|| read_auth(&path)).ok();
        let email = auth
            .as_ref()
            .and_then(extract_email)
            .unwrap_or_else(|| "?".to_string());
        let usage = fetch_rate_limit_for_auth_path(&path).map(|(usage, _)| usage);
        let account_id = auth
            .as_ref()
            .and_then(|auth| auth.tokens.account_id.clone());
        entries.push(ProfileUsage {
            provider: "codex",
            session_id: format!("codex:profile:{}", name),
            name,
            email,
            account_id,
            is_live: false,
            usage,
        });
    }

    if let Some(pi_auth) = live_pi.as_ref() {
        let usage = fetch_pi_rate_limit(ctx, pi_auth.clone()).map(|(usage, _)| usage);
        entries.push(ProfileUsage {
            provider: "pi",
            session_id: "pi:openai-codex".to_string(),
            name: detect_pi_profile(ctx, pi_auth),
            email: extract_email_from_token(&pi_auth.access).unwrap_or_else(|| "?".to_string()),
            account_id: pi_auth.account_id.clone(),
            is_live: true,
            usage,
        });
    }

    for path in list_pi_profiles(ctx) {
        let Some(pi_auth) = read_pi_auth(&path).and_then(|auth| auth.openai_codex) else {
            continue;
        };
        if live_pi
            .as_ref()
            .is_some_and(|live| same_pi_account(live, &pi_auth))
        {
            continue;
        }
        let name = profile_name(&path);
        let usage = fetch_pi_rate_limit_for_path(&path, pi_auth.clone()).map(|(usage, _)| usage);
        entries.push(ProfileUsage {
            provider: "pi",
            session_id: format!("pi:profile:{}", name),
            name,
            email: extract_email_from_token(&pi_auth.access).unwrap_or_else(|| "?".to_string()),
            account_id: pi_auth.account_id.clone(),
            is_live: false,
            usage,
        });
    }

    entries
}

fn same_pi_account(a: &PiOpenAiCodexAuth, b: &PiOpenAiCodexAuth) -> bool {
    if a.account_id.is_some() && a.account_id == b.account_id {
        return true;
    }
    let a_email = extract_email_from_token(&a.access);
    a_email.is_some() && a_email == extract_email_from_token(&b.access)
}

fn detect_pi_profile(ctx: &Context, pi_auth: &PiOpenAiCodexAuth) -> String {
    let pi_email = extract_email_from_token(&pi_auth.access);
    for path in list_pi_profiles(ctx) {
        let Some(profile_pi_auth) = read_pi_auth(&path).and_then(|auth| auth.openai_codex) else {
            continue;
        };
        if pi_auth.account_id.is_some() && profile_pi_auth.account_id == pi_auth.account_id {
            return profile_name(&path);
        }
        if pi_email.is_some() && extract_email_from_token(&profile_pi_auth.access) == pi_email {
            return profile_name(&path);
        }
    }
    "live".to_string()
}

fn entry_percentage(entry: &ProfileUsage) -> Option<u8> {
    let usage = entry.usage.as_ref().ok()?;
    usage
        .monthly_limit()
        .and_then(|limit| limit.used_percent)
        .or_else(|| {
            usage
                .five_hour_window()
                .or_else(|| usage.weekly_window())
                .and_then(|window| window.used_percent)
        })
        .map(|value| value.clamp(0.0, 100.0).round() as u8)
}

fn class_for_percentage(percentage: u8) -> &'static str {
    match percentage {
        90..=100 => "critical",
        80..=89 => "warning",
        _ => "ok",
    }
}

fn update_last_quota_hit(ctx: &Context, entries: &[ProfileUsage]) -> Option<TrackedQuotaHit> {
    let mut tracker = load_tracker(ctx);
    let mut last_hit = tracker.last_quota_hit.clone();
    let observed_at = Utc::now().to_rfc3339();

    for entry in entries {
        let Ok(usage) = &entry.usage else { continue };
        let previous_session = tracker
            .sessions
            .iter()
            .find(|session| session.session_id == entry.session_id);
        let previous_rate = previous_session.and_then(|session| session.rate_limit.as_ref());
        let previous_monthly = previous_session.and_then(|session| session.monthly_usage.as_ref());

        let primary = usage.five_hour_window();
        let secondary = usage.weekly_window();
        let monthly = usage.monthly_limit();
        let primary_used = primary.and_then(|window| window.used_percent);
        let secondary_used = secondary.and_then(|window| window.used_percent);
        let monthly_used = monthly.and_then(|limit| limit.used_percent);
        let mut hit_window = None;
        let mut previous_used = None;
        let mut current_used = None;

        if usage_increased(
            previous_monthly.and_then(|value| value.used_percent),
            monthly_used,
        ) {
            hit_window = Some("month".to_string());
            previous_used = previous_monthly.and_then(|value| value.used_percent);
            current_used = monthly_used;
        } else if usage_increased(
            previous_rate.and_then(|rate| rate.used_percent),
            primary_used,
        ) {
            hit_window = Some("5h".to_string());
            previous_used = previous_rate.and_then(|rate| rate.used_percent);
            current_used = primary_used;
        } else if usage_increased(
            previous_rate.and_then(|rate| rate.secondary_used_percent),
            secondary_used,
        ) {
            hit_window = Some("7d".to_string());
            previous_used = previous_rate.and_then(|rate| rate.secondary_used_percent);
            current_used = secondary_used;
        }

        if let Some(window) = hit_window {
            last_hit = Some(TrackedQuotaHit {
                observed_at: Some(observed_at.clone()),
                provider: Some(entry.provider.to_string()),
                session_id: Some(entry.session_id.clone()),
                profile: Some(entry.name.clone()),
                email: Some(entry.email.clone()),
                account_id: entry.account_id.clone(),
                window: Some(window),
                previous_used_percent: previous_used,
                used_percent: current_used,
            });
        }

        let resets_at = primary
            .and_then(|window| parse_reset_at(window.reset_at.as_ref()))
            .unwrap_or(0);
        let secondary_resets_at =
            secondary.and_then(|window| parse_reset_at(window.reset_at.as_ref()));

        let session = crate::tracker::upsert_session(
            &mut tracker,
            &entry.session_id,
            Some(entry.provider.to_string()),
            None,
            entry.account_id.as_deref().unwrap_or(""),
            Some(entry.name.clone()),
            Some(entry.email.clone()),
            None,
            None,
            None,
            false,
            None,
        );
        if let Some(monthly) = monthly {
            update_monthly_usage(
                session,
                Some(observed_at.clone()),
                monthly.limit.as_ref().and_then(|value| value.as_f64()),
                monthly.used.as_ref().and_then(|value| value.as_f64()),
                monthly.remaining.as_ref().and_then(|value| value.as_f64()),
                monthly.used_percent,
                monthly.remaining_percent,
                parse_reset_at(monthly.reset_at.as_ref()),
                usage.spend_control.as_ref().and_then(|value| value.reached),
                usage.plan_type.clone(),
            );
        } else if primary.is_some() || secondary.is_some() {
            update_rate_limit(
                session,
                Some(observed_at.clone()),
                primary_used,
                resets_at,
                secondary_used,
                secondary_resets_at,
                usage.plan_type.clone(),
            );
        }
    }

    tracker.last_quota_hit = last_hit.clone();
    save_tracker(ctx, &tracker);
    last_hit
}

fn usage_increased(previous: Option<f64>, current: Option<f64>) -> bool {
    match (previous, current) {
        (Some(previous), Some(current)) => current > previous + 0.01,
        _ => false,
    }
}

fn format_entry(
    template: &str,
    entry: &ProfileUsage,
    last_hit: Option<&TrackedQuotaHit>,
) -> String {
    format_entry_with_options(template, entry, last_hit, false, false, false)
}

fn format_entry_with_options(
    template: &str,
    entry: &ProfileUsage,
    last_hit: Option<&TrackedQuotaHit>,
    hide_minutes_with_days: bool,
    hide_hours_with_days: bool,
    percent_left: bool,
) -> String {
    let mut values = FormatValues::default();
    if let Ok(usage) = &entry.usage {
        if let Some(window) = usage.five_hour_window() {
            if let Some((used, reset_in, _)) = summarize_window(window) {
                values.five_hour_used_pct = used.clone();
                values.five_hour_remaining_pct = window
                    .used_percent
                    .map(remaining_percentage)
                    .unwrap_or_default();
                values.five_hour_pct = if percent_left {
                    values.five_hour_remaining_pct.clone()
                } else {
                    used
                };
                values.five_hour_reset =
                    compact_reset(reset_in, hide_minutes_with_days, hide_hours_with_days);
            }
        }
        if let Some(window) = usage.weekly_window() {
            if let Some((used, reset_in, _)) = summarize_window(window) {
                values.seven_day_used_pct = used.clone();
                values.seven_day_remaining_pct = window
                    .used_percent
                    .map(remaining_percentage)
                    .unwrap_or_default();
                values.seven_day_pct = if percent_left {
                    values.seven_day_remaining_pct.clone()
                } else {
                    used
                };
                values.seven_day_reset =
                    compact_reset(reset_in, hide_minutes_with_days, hide_hours_with_days);
            }
        }
        if let Some(monthly) = usage.monthly_limit() {
            values.monthly_limit = format_credit_amount(monthly.limit.as_ref());
            values.monthly_used = format_credit_amount(monthly.used.as_ref());
            values.monthly_remaining = format_credit_amount(monthly.remaining.as_ref());
            values.monthly_used_pct = monthly
                .used_percent
                .or_else(|| monthly.remaining_percent.map(complement_percentage))
                .map(format_percentage)
                .unwrap_or_default();
            values.monthly_remaining_pct = monthly
                .remaining_percent
                .or_else(|| monthly.used_percent.map(complement_percentage))
                .map(format_percentage)
                .unwrap_or_default();
            values.monthly_pct = if percent_left {
                values.monthly_remaining_pct.clone()
            } else {
                values.monthly_used_pct.clone()
            };
            if let Some((reset_in, _)) = summarize_reset(monthly.reset_at.as_ref()) {
                values.monthly_reset =
                    compact_reset(reset_in, hide_minutes_with_days, hide_hours_with_days);
            }
        }
        if let Some(reset_credits) = usage.rate_limit_reset_credits.as_ref() {
            values.available_resets = reset_credits
                .available_count
                .map(|value| value.to_string())
                .unwrap_or_default();
            values.applicable_resets = reset_credits
                .applicable_available_count
                .map(|value| value.to_string())
                .unwrap_or_default();
            if let Some((reset_in, reset_at)) = earliest_reset_credit_expiry(reset_credits) {
                values.reset_expiry =
                    compact_reset(reset_in, hide_minutes_with_days, hide_hours_with_days);
                values.reset_expiry_at = reset_at;
            }
        }
    }
    values.status = if entry.usage.is_ok() {
        "ok"
    } else {
        "unavailable"
    }
    .to_string();
    values.profile = entry.name.clone();
    values.provider = entry.provider.to_string();
    values.email = entry.email.clone();
    apply_last_hit(&mut values, last_hit);
    if values.monthly_limit.is_empty() {
        if values.five_hour_pct.is_empty() {
            values.window = "7d".to_string();
            values.percent = values.seven_day_pct.clone();
            values.reset = values.seven_day_reset.clone();
        } else {
            values.window = "5h".to_string();
            values.percent = values.five_hour_pct.clone();
            values.reset = values.five_hour_reset.clone();
        }
    } else {
        values.window = "month".to_string();
        values.percent = values.monthly_pct.clone();
        values.reset = values.monthly_reset.clone();
    }
    values.apply(template)
}

fn format_percentage(value: f64) -> String {
    format!("{}", value)
}

fn complement_percentage(value: f64) -> f64 {
    (100.0 - value).clamp(0.0, 100.0)
}

fn remaining_percentage(used_percent: f64) -> String {
    format_percentage(complement_percentage(used_percent))
}

fn compact_reset(
    reset: String,
    hide_minutes_with_days: bool,
    hide_hours_with_days: bool,
) -> String {
    if !reset.split_whitespace().any(|part| part.ends_with('d')) {
        return reset;
    }
    reset
        .split_whitespace()
        .filter(|part| {
            !(hide_minutes_with_days && part.ends_with('m'))
                && !(hide_hours_with_days && part.ends_with('h'))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_reset_credit_expiry(expires_at: &str) -> Option<(u64, String, String)> {
    let parsed = DateTime::parse_from_rfc3339(expires_at).ok()?;
    let timestamp = u64::try_from(parsed.timestamp()).ok()?;
    let duration = format_duration_until(timestamp);
    let local = parsed
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S %Z")
        .to_string();
    Some((timestamp, duration, local))
}

fn earliest_reset_credit_expiry(
    reset_credits: &crate::data::RateLimitResetCredits,
) -> Option<(String, String)> {
    reset_credits
        .credits
        .iter()
        .filter(|credit| credit.status.as_deref() == Some("available"))
        .filter_map(|credit| {
            credit
                .expires_at
                .as_deref()
                .and_then(format_reset_credit_expiry)
        })
        .min_by_key(|(timestamp, _, _)| *timestamp)
        .map(|(_, duration, local)| (duration, local))
}

#[derive(Default)]
struct FormatValues {
    five_hour_pct: String,
    five_hour_used_pct: String,
    five_hour_remaining_pct: String,
    seven_day_pct: String,
    seven_day_used_pct: String,
    seven_day_remaining_pct: String,
    five_hour_reset: String,
    seven_day_reset: String,
    monthly_limit: String,
    monthly_used: String,
    monthly_remaining: String,
    monthly_used_pct: String,
    monthly_remaining_pct: String,
    monthly_pct: String,
    monthly_reset: String,
    available_resets: String,
    applicable_resets: String,
    reset_expiry: String,
    reset_expiry_at: String,
    status: String,
    profile: String,
    provider: String,
    email: String,
    window: String,
    percent: String,
    reset: String,
    last_hit_provider: String,
    last_hit_profile: String,
    last_hit_email: String,
    last_hit_window: String,
    last_hit_pct: String,
    last_hit_at: String,
}

fn apply_last_hit(values: &mut FormatValues, last_hit: Option<&TrackedQuotaHit>) {
    let Some(last_hit) = last_hit else { return };
    values.last_hit_provider = last_hit.provider.clone().unwrap_or_default();
    values.last_hit_profile = last_hit.profile.clone().unwrap_or_default();
    values.last_hit_email = last_hit.email.clone().unwrap_or_default();
    values.last_hit_window = last_hit.window.clone().unwrap_or_default();
    values.last_hit_pct = last_hit
        .used_percent
        .map(|value| format!("{:.0}", value))
        .unwrap_or_default();
    values.last_hit_at = last_hit.observed_at.clone().unwrap_or_default();
}

impl FormatValues {
    fn apply(&self, template: &str) -> String {
        let five_hour_block = if self.five_hour_pct.is_empty() {
            String::new()
        } else {
            format!(
                "{} {}% {} {} ",
                ICON, self.five_hour_pct, TIME_ICON, self.five_hour_reset
            )
        };
        let five_hour_block_pango = if self.five_hour_pct.is_empty() {
            String::new()
        } else {
            format!(
                "<span font_family=\"bootstrap-icons\" rise=\"1200\" color=\"#5f78ff\">{}</span> {}% <span color=\"#5f78ff\">{}</span> {} ",
                ICON, self.five_hour_pct, TIME_ICON, self.five_hour_reset
            )
        };
        let usage_block = if self.monthly_limit.is_empty() {
            format!(
                "{}{} {} {} {}",
                five_hour_block,
                ICON,
                value_or_unknown(&self.seven_day_pct),
                TIME_ICON,
                value_or_unknown(&self.seven_day_reset)
            )
        } else {
            format!(
                "{} {}% {} {}",
                ICON,
                value_or_unknown(&self.monthly_pct),
                TIME_ICON,
                value_or_unknown(&self.monthly_reset)
            )
        };
        let usage_block_pango = if self.monthly_limit.is_empty() {
            format!(
                "{}<span font_family=\"bootstrap-icons\" rise=\"1200\" color=\"#5f78ff\">{}</span> {} <span color=\"#5f78ff\">{}</span> {}",
                five_hour_block_pango,
                ICON,
                value_or_unknown(&self.seven_day_pct),
                TIME_ICON,
                value_or_unknown(&self.seven_day_reset)
            )
        } else {
            format!(
                "<span font_family=\"bootstrap-icons\" rise=\"1200\" color=\"#5f78ff\">{}</span> {}% <span color=\"#5f78ff\">{}</span> {}",
                ICON,
                value_or_unknown(&self.monthly_pct),
                TIME_ICON,
                value_or_unknown(&self.monthly_reset)
            )
        };

        template
            .replace("{usage_block_pango}", &usage_block_pango)
            .replace("{usage_block}", &usage_block)
            .replace("{5h_block_pango}", &five_hour_block_pango)
            .replace("{5h_block}", &five_hour_block)
            .replace("{icon}", ICON)
            .replace("{icon_plain}", ICON)
            .replace("{time_icon}", TIME_ICON)
            .replace("{time_icon_plain}", TIME_ICON)
            .replace("{5h_pct}", value_or_unknown(&self.five_hour_pct))
            .replace("{5h_used_pct}", value_or_unknown(&self.five_hour_used_pct))
            .replace(
                "{5h_remaining_pct}",
                value_or_unknown(&self.five_hour_remaining_pct),
            )
            .replace("{7d_pct}", value_or_unknown(&self.seven_day_pct))
            .replace("{7d_used_pct}", value_or_unknown(&self.seven_day_used_pct))
            .replace(
                "{7d_remaining_pct}",
                value_or_unknown(&self.seven_day_remaining_pct),
            )
            .replace("{5h_reset}", value_or_unknown(&self.five_hour_reset))
            .replace("{7d_reset}", value_or_unknown(&self.seven_day_reset))
            .replace("{monthly_limit}", value_or_unknown(&self.monthly_limit))
            .replace("{monthly_used}", value_or_unknown(&self.monthly_used))
            .replace(
                "{monthly_remaining}",
                value_or_unknown(&self.monthly_remaining),
            )
            .replace(
                "{monthly_used_pct}",
                value_or_unknown(&self.monthly_used_pct),
            )
            .replace(
                "{monthly_remaining_pct}",
                value_or_unknown(&self.monthly_remaining_pct),
            )
            .replace("{monthly_pct}", value_or_unknown(&self.monthly_pct))
            .replace("{monthly_reset}", value_or_unknown(&self.monthly_reset))
            .replace(
                "{available_resets}",
                value_or_unknown(&self.available_resets),
            )
            .replace(
                "{applicable_resets}",
                value_or_unknown(&self.applicable_resets),
            )
            .replace("{reset_expiry}", value_or_unknown(&self.reset_expiry))
            .replace("{reset_expiry_at}", value_or_unknown(&self.reset_expiry_at))
            .replace("{status}", &self.status)
            .replace("{profile}", &self.profile)
            .replace("{provider}", &self.provider)
            .replace("{email}", &self.email)
            .replace("{win}", &self.window)
            .replace("{pct}", value_or_unknown(&self.percent))
            .replace("{reset}", value_or_unknown(&self.reset))
            .replace(
                "{last_hit_provider}",
                value_or_unknown(&self.last_hit_provider),
            )
            .replace(
                "{last_hit_profile}",
                value_or_unknown(&self.last_hit_profile),
            )
            .replace("{last_hit_email}", value_or_unknown(&self.last_hit_email))
            .replace("{last_hit_window}", value_or_unknown(&self.last_hit_window))
            .replace("{last_hit_pct}", value_or_unknown(&self.last_hit_pct))
            .replace("{last_hit_at}", value_or_unknown(&self.last_hit_at))
    }
}

fn value_or_unknown(value: &str) -> &str {
    if value.is_empty() {
        "?"
    } else {
        value
    }
}

fn same_usage_account(a: &ProfileUsage, b: &ProfileUsage) -> bool {
    match (&a.account_id, &b.account_id) {
        (Some(a_id), Some(b_id)) => a_id == b_id,
        _ => a.email != "?" && a.email == b.email,
    }
}

fn unique_usage_accounts(entries: &[ProfileUsage]) -> Vec<&ProfileUsage> {
    let mut unique: Vec<&ProfileUsage> = Vec::new();
    for entry in entries {
        if let Some(existing) = unique
            .iter_mut()
            .find(|existing| same_usage_account(existing, entry))
        {
            if existing.usage.is_err() && entry.usage.is_ok() {
                *existing = entry;
            }
        } else {
            unique.push(entry);
        }
    }
    unique
}

fn append_reset_credit_tooltip(lines: &mut Vec<String>, usage: &UsageResponse) {
    let Some(reset_credits) = usage.rate_limit_reset_credits.as_ref() else {
        return;
    };
    let available = reset_credits.available_count.unwrap_or(0);
    if available == 0 {
        return;
    }

    let applicable = reset_credits
        .applicable_available_count
        .map(|count| count.to_string())
        .unwrap_or_else(|| "?".to_string());
    lines.push(format!(
        "  Reset credits: {} available ({} currently applicable)",
        available, applicable
    ));

    for credit in reset_credits
        .credits
        .iter()
        .filter(|credit| credit.status.as_deref() == Some("available"))
    {
        let title = credit.title.as_deref().unwrap_or("Reset");
        match credit
            .expires_at
            .as_deref()
            .and_then(format_reset_credit_expiry)
        {
            Some((_, duration, local)) => lines.push(format!(
                "    {}: expires in {} ({})",
                title, duration, local
            )),
            None => lines.push(format!("    {}: expiration unavailable", title)),
        }
    }
}

fn format_tooltip(entries: &[ProfileUsage], last_hit: Option<&TrackedQuotaHit>) -> String {
    if entries.is_empty() {
        return "No accounts found".to_string();
    }

    let mut lines = vec!["Usage".to_string()];
    if let Some(hit) = last_hit {
        lines.push(format!(
            "Last quota hit: {} ({}) {}% {} at {}",
            hit.profile.as_deref().unwrap_or("?"),
            hit.email.as_deref().unwrap_or("?"),
            hit.used_percent
                .map(|value| format!("{:.0}", value))
                .unwrap_or_else(|| "?".to_string()),
            hit.window.as_deref().unwrap_or("?"),
            hit.observed_at.as_deref().unwrap_or("?")
        ));
    }
    for entry in unique_usage_accounts(entries) {
        match &entry.usage {
            Ok(usage) if usage.monthly_limit().is_some() => lines.push(format!(
                "{}: month {} / {} credits used ({}%) | {} credits left ({}%) | reset {} | limit reached {}",
                entry.name,
                format_entry("{monthly_used}", entry, last_hit),
                format_entry("{monthly_limit}", entry, last_hit),
                format_entry("{monthly_used_pct}", entry, last_hit),
                format_entry("{monthly_remaining}", entry, last_hit),
                format_entry("{monthly_remaining_pct}", entry, last_hit),
                format_entry("{monthly_reset}", entry, last_hit),
                usage
                    .spend_control
                    .as_ref()
                    .and_then(|value| value.reached)
                    .map(|value| if value { "yes" } else { "no" })
                    .unwrap_or("?"),
            )),
            Ok(_) => {
                let five_hour_pct = format_entry("{5h_pct}", entry, last_hit);
                let five_hour_reset = format_entry("{5h_reset}", entry, last_hit);
                let mut windows = Vec::new();
                if five_hour_pct != "?" || five_hour_reset != "?" {
                    windows.push(format!(
                        "5h {}% reset {}",
                        five_hour_pct, five_hour_reset
                    ));
                }
                windows.push(format!(
                    "7d {}% reset {}",
                    format_entry("{7d_pct}", entry, last_hit),
                    format_entry("{7d_reset}", entry, last_hit),
                ));
                lines.push(format!("{}: {}", entry.name, windows.join(" | ")));
            }
            Err(err) => lines.push(format!("{}: unavailable ({})", entry.name, err)),
        }
        if let Ok(usage) = &entry.usage {
            append_reset_credit_tooltip(&mut lines, usage);
        }
    }
    lines.join("\n")
}

fn format_alt(
    entries: &[ProfileUsage],
    last_hit: Option<&TrackedQuotaHit>,
    percent_left: bool,
) -> String {
    entries
        .iter()
        .map(|entry| {
            format!(
                "{}:{}:{}:{}:{}",
                entry.provider,
                entry.name,
                format_entry_with_options("{win}", entry, last_hit, false, false, percent_left),
                format_entry_with_options("{pct}", entry, last_hit, false, false, percent_left),
                format_entry_with_options("{reset}", entry, last_hit, false, false, percent_left)
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::{
        apply_last_hit, class_for_percentage, compact_reset, display_entry, entry_percentage,
        format_entry, format_entry_with_options, format_tooltip, same_pi_account, FormatValues,
        ProfileUsage, DEFAULT_FORMAT, ICON, TIME_ICON,
    };
    use crate::data::{PiOpenAiCodexAuth, TrackedQuotaHit, UsageResponse};

    #[test]
    fn waybar_format_replaces_codex_usage_tokens() {
        let values = FormatValues {
            five_hour_pct: "42".to_string(),
            seven_day_pct: "12".to_string(),
            five_hour_reset: "1h 2m".to_string(),
            seven_day_reset: "4d 3h 2m".to_string(),
            status: "ok".to_string(),
            profile: "me".to_string(),
            provider: "codex".to_string(),
            email: "me@example.com".to_string(),
            window: "5h".to_string(),
            percent: "42".to_string(),
            reset: "1h 2m".to_string(),
            ..FormatValues::default()
        };

        assert_eq!(
            values.apply("{provider} {profile} {5h_pct}% {5h_reset} {7d_pct}% {7d_reset}"),
            "codex me 42% 1h 2m 12% 4d 3h 2m"
        );
    }

    #[test]
    fn waybar_percent_left_mode_applies_to_subscription_windows() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{"rate_limit":{"primary_window":{"used_percent":42,"limit_window_seconds":18000,"reset_at":4102444800},"secondary_window":{"used_percent":12,"limit_window_seconds":604800,"reset_at":4102444800}}}"#,
        )
        .unwrap();
        let entry = ProfileUsage {
            provider: "codex",
            session_id: "codex:live".to_string(),
            name: "me".to_string(),
            email: "me@example.com".to_string(),
            account_id: Some("acct-me".to_string()),
            is_live: true,
            usage: Ok(usage),
        };

        assert_eq!(
            format_entry_with_options(
                "{5h_pct}/{7d_pct} {5h_used_pct}/{5h_remaining_pct} {7d_used_pct}/{7d_remaining_pct} {pct}",
                &entry,
                None,
                false,
                false,
                true,
            ),
            "58/88 42/58 12/88 58"
        );
        let text = format_entry_with_options(DEFAULT_FORMAT, &entry, None, false, false, true);
        assert!(text.contains("58%"));
        assert!(text.contains("88"));
        assert!(!text.contains("left"));
    }

    #[test]
    fn waybar_omits_five_hour_block_when_window_is_absent() {
        let values = FormatValues {
            seven_day_pct: "11".to_string(),
            seven_day_reset: "6d 23h 3m".to_string(),
            ..FormatValues::default()
        };

        assert_eq!(
            values.apply(DEFAULT_FORMAT),
            format!("{} 11 {} 6d 23h 3m", ICON, TIME_ICON)
        );
        assert_eq!(values.apply("{5h_block_pango}weekly"), "weekly");
    }

    #[test]
    fn compact_reset_hides_minutes_only_when_days_are_present() {
        assert_eq!(compact_reset("6d 23h 3m".to_string(), true, true), "6d");
        assert_eq!(compact_reset("23h 3m".to_string(), true, true), "23h 3m");
        assert_eq!(
            compact_reset("6d 23h 3m".to_string(), true, false),
            "6d 23h"
        );
        assert_eq!(compact_reset("6d 23h 3m".to_string(), false, true), "6d 3m");
        assert_eq!(
            compact_reset("6d 23h 3m".to_string(), false, false),
            "6d 23h 3m"
        );
    }

    #[test]
    fn waybar_class_reflects_active_usage() {
        assert_eq!(class_for_percentage(79), "ok");
        assert_eq!(class_for_percentage(80), "warning");
        assert_eq!(class_for_percentage(90), "critical");
    }

    #[test]
    fn waybar_formats_business_monthly_usage_and_tooltip() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{
                "plan_type":"business",
                "rate_limit":null,
                "spend_control":{"reached":false,"individual_limit":{
                    "limit":"12500","used":"94.4807","remaining":"12405.5193",
                    "used_percent":1,"remaining_percent":99,"reset_at":4102444800
                }}
            }"#,
        )
        .unwrap();
        let entries = vec![ProfileUsage {
            provider: "codex",
            session_id: "codex:live".to_string(),
            name: "work".to_string(),
            email: "person@example.com".to_string(),
            account_id: Some("acct-work".to_string()),
            is_live: true,
            usage: Ok(usage),
        }];
        let entry = &entries[0];

        let text = format_entry(DEFAULT_FORMAT, entry, None);
        assert!(text.contains(&format!("{} 1% {}", ICON, TIME_ICON)));
        assert!(!text.contains("left"));
        let left_text = format_entry_with_options(DEFAULT_FORMAT, entry, None, false, false, true);
        assert!(left_text.contains(&format!("{} 99% {}", ICON, TIME_ICON)));
        assert!(!left_text.contains("left"));
        assert_eq!(
            format_entry_with_options(
                "{pct} {monthly_pct} {monthly_used_pct} {monthly_remaining_pct}",
                entry,
                None,
                false,
                false,
                true,
            ),
            "99 99 1 99"
        );
        assert_eq!(entry_percentage(entry), Some(1));
        assert_eq!(
            format_entry(
                "{win} {monthly_used}/{monthly_limit} {monthly_remaining} {monthly_used_pct}% {monthly_remaining_pct}%",
                entry,
                None,
            ),
            "month 94.48/12500 12405.52 1% 99%"
        );
        let tooltip = format_tooltip(&entries, None);
        assert!(tooltip.contains("month 94.48 / 12500 credits used (1%)"));
        assert!(tooltip.contains("12405.52 credits left (99%)"));
        assert!(tooltip.contains("limit reached no"));
    }

    #[test]
    fn waybar_formats_enterprise_used_and_remaining_percentages() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{"plan_type":"enterprise","spend_control":{"individual_limit":{"remaining_percent":10,"reset_at":4102444800}}}"#,
        )
        .unwrap();
        let entry = ProfileUsage {
            provider: "codex",
            session_id: "codex:live".to_string(),
            name: "work".to_string(),
            email: "person@example.com".to_string(),
            account_id: Some("acct-work".to_string()),
            is_live: true,
            usage: Ok(usage),
        };

        let used = format_entry_with_options(DEFAULT_FORMAT, &entry, None, false, false, false);
        let remaining = format_entry_with_options(DEFAULT_FORMAT, &entry, None, false, false, true);
        assert!(used.contains(&format!("{} 90% {}", ICON, TIME_ICON)));
        assert!(remaining.contains(&format!("{} 10% {}", ICON, TIME_ICON)));
        assert!(!used.contains("left"));
        assert!(!remaining.contains("left"));
    }

    #[test]
    fn waybar_formats_available_resets_and_expiration() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{
                "rate_limit":{"primary_window":{"used_percent":5,"limit_window_seconds":604800,"reset_at":4102444800}},
                "rate_limit_reset_credits":{
                    "available_count":1,
                    "applicable_available_count":0,
                    "credits":[{
                        "status":"available",
                        "title":"Full reset",
                        "expires_at":"2100-01-01T12:00:00Z"
                    }]
                }
            }"#,
        )
        .unwrap();
        let entries = vec![ProfileUsage {
            provider: "codex",
            session_id: "codex:live".to_string(),
            name: "me".to_string(),
            email: "me@example.com".to_string(),
            account_id: Some("acct-me".to_string()),
            is_live: true,
            usage: Ok(usage),
        }];
        let entry = &entries[0];

        let formatted = format_entry(
            "{available_resets}/{applicable_resets} {reset_expiry} {reset_expiry_at}",
            entry,
            None,
        );
        assert!(formatted.starts_with("1/0 "));
        assert!(formatted.contains("2100-01-01"));
        assert!(!formatted.contains('?'));

        let tooltip = format_tooltip(&entries, None);
        assert!(tooltip.contains("Reset credits: 1 available (0 currently applicable)"));
        assert!(tooltip.contains("Full reset: expires in"));
        assert!(tooltip.contains("2100-01-01"));
    }

    #[test]
    fn waybar_tooltip_deduplicates_accounts_without_app_names() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{
                "plan_type":"business",
                "rate_limit":null,
                "spend_control":{"reached":false,"individual_limit":{
                    "limit":"12500","used":"94.4807","remaining":"12405.5193",
                    "used_percent":1,"remaining_percent":99,"reset_at":4102444800
                }}
            }"#,
        )
        .unwrap();
        let entries = vec![
            ProfileUsage {
                provider: "codex",
                session_id: "codex:live".to_string(),
                name: "me".to_string(),
                email: "me@example.com".to_string(),
                account_id: Some("acct-me".to_string()),
                is_live: true,
                usage: Err("failed".to_string()),
            },
            ProfileUsage {
                provider: "pi",
                session_id: "pi:live".to_string(),
                name: "me".to_string(),
                email: "me@example.com".to_string(),
                account_id: Some("acct-me".to_string()),
                is_live: true,
                usage: Ok(usage),
            },
            ProfileUsage {
                provider: "codex",
                session_id: "codex:profile:mate".to_string(),
                name: "mate".to_string(),
                email: "mate@example.com".to_string(),
                account_id: None,
                is_live: false,
                usage: Err("unavailable".to_string()),
            },
            ProfileUsage {
                provider: "pi",
                session_id: "pi:profile:mate".to_string(),
                name: "mate".to_string(),
                email: "mate@example.com".to_string(),
                account_id: None,
                is_live: false,
                usage: Err("unavailable".to_string()),
            },
        ];

        let tooltip = format_tooltip(&entries, None);
        assert!(tooltip.starts_with("Usage\n"));
        assert_eq!(tooltip.matches("me:").count(), 1);
        assert_eq!(tooltip.matches("mate:").count(), 1);
        assert!(!tooltip.contains("codex "));
        assert!(!tooltip.contains("pi "));
        assert!(tooltip.contains("me: month 94.48 / 12500 credits used"));
    }

    #[test]
    fn waybar_tooltip_hides_disabled_five_hour_window() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{"rate_limit":{"primary_window":{"used_percent":8,"limit_window_seconds":604800,"reset_at":4102444800},"secondary_window":null}}"#,
        )
        .unwrap();
        let entries = vec![ProfileUsage {
            provider: "codex",
            session_id: "codex:live".to_string(),
            name: "me".to_string(),
            email: "me@example.com".to_string(),
            account_id: Some("acct-me".to_string()),
            is_live: true,
            usage: Ok(usage),
        }];

        let tooltip = format_tooltip(&entries, None);
        assert!(!tooltip.contains("me: 5h"));
        assert!(!tooltip.contains("| 5h"));
        assert!(tooltip.contains("me: 7d 8% reset"));
        assert_eq!(
            format_entry_with_options("{win} {pct}", &entries[0], None, false, false, true),
            "7d 92"
        );
    }

    #[test]
    fn waybar_tooltip_last_hit_omits_provider() {
        let entries = vec![ProfileUsage {
            provider: "codex",
            session_id: "codex:live".to_string(),
            name: "me".to_string(),
            email: "me@example.com".to_string(),
            account_id: Some("acct-me".to_string()),
            is_live: true,
            usage: Err("failed".to_string()),
        }];
        let hit = TrackedQuotaHit {
            provider: Some("codex".to_string()),
            profile: Some("me".to_string()),
            email: Some("me@example.com".to_string()),
            window: Some("7d".to_string()),
            used_percent: Some(42.0),
            observed_at: Some("2026-07-27T00:00:00Z".to_string()),
            ..TrackedQuotaHit::default()
        };

        let tooltip = format_tooltip(&entries, Some(&hit));
        assert!(
            tooltip.contains("Last quota hit: me (me@example.com) 42% 7d at 2026-07-27T00:00:00Z")
        );
        assert!(!tooltip.contains("Last quota hit: codex"));
    }

    #[test]
    fn waybar_displays_last_hit_entry_when_available() {
        let entries = vec![
            ProfileUsage {
                provider: "codex",
                session_id: "codex:live".to_string(),
                name: "mate".to_string(),
                email: "mate@example.com".to_string(),
                account_id: Some("acct-mate".to_string()),
                is_live: true,
                usage: Err("unused".to_string()),
            },
            ProfileUsage {
                provider: "pi",
                session_id: "pi:profile:me".to_string(),
                name: "me".to_string(),
                email: "me@example.com".to_string(),
                account_id: Some("acct-me".to_string()),
                is_live: false,
                usage: Err("unused".to_string()),
            },
        ];
        let hit = TrackedQuotaHit {
            session_id: Some("pi:profile:me".to_string()),
            ..TrackedQuotaHit::default()
        };

        assert_eq!(display_entry(&entries, Some(&hit)).unwrap().name, "me");
    }

    #[test]
    fn same_pi_account_matches_account_id_even_when_tokens_differ() {
        let live = PiOpenAiCodexAuth {
            auth_type: Some("oauth".to_string()),
            access: "new-access".to_string(),
            refresh: Some("new-refresh".to_string()),
            account_id: Some("acct-me".to_string()),
            expires: None,
        };
        let profile = PiOpenAiCodexAuth {
            auth_type: Some("oauth".to_string()),
            access: "old-access".to_string(),
            refresh: Some("old-refresh".to_string()),
            account_id: Some("acct-me".to_string()),
            expires: None,
        };

        assert!(same_pi_account(&live, &profile));
    }

    #[test]
    fn waybar_format_replaces_last_quota_hit_tokens() {
        let hit = TrackedQuotaHit {
            provider: Some("codex".to_string()),
            profile: Some("work".to_string()),
            email: Some("person@example.com".to_string()),
            window: Some("5h".to_string()),
            used_percent: Some(42.0),
            observed_at: Some("2026-07-04T00:00:00Z".to_string()),
            ..TrackedQuotaHit::default()
        };
        let mut values = FormatValues::default();
        apply_last_hit(&mut values, Some(&hit));

        assert_eq!(
            values.apply("{last_hit_provider} {last_hit_profile} {last_hit_email} {last_hit_window} {last_hit_pct} {last_hit_at}"),
            "codex work person@example.com 5h 42 2026-07-04T00:00:00Z"
        );
    }
}

use crate::data::{read_auth, read_pi_auth, Context, PiOpenAiCodexAuth, UsageResponse};
use crate::jwt::{extract_email, extract_email_from_token};
use crate::profile::{detect_current_profile, list_pi_profiles, list_profiles, profile_name};
use crate::rate_limit::{
    fetch_pi_rate_limit, fetch_pi_rate_limit_for_path, fetch_rate_limit_for_auth_path,
    summarize_window,
};
use serde::Serialize;

const DEFAULT_FORMAT: &str = "{icon_plain} {5h_pct}% {time_icon_plain} {5h_reset} {icon_plain} {7d_pct}% {time_icon_plain} {7d_reset}";
const ICON: &str = "\u{F915}";
const TIME_ICON: &str = "󰥔";

#[derive(Debug)]
struct ProfileUsage {
    provider: &'static str,
    name: String,
    email: String,
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

pub fn print_waybar(ctx: &Context, format: Option<&str>, tooltip_format: Option<&str>) {
    let entries = collect_profile_usage(ctx);
    let active = entries
        .iter()
        .find(|entry| entry.provider == "codex" && entry.is_live)
        .or_else(|| entries.iter().find(|entry| entry.is_live))
        .or_else(|| entries.first());

    let text = active
        .map(|entry| format_entry(format.unwrap_or(DEFAULT_FORMAT), entry))
        .unwrap_or_else(|| "codex ?".to_string());
    let percentage = active.and_then(entry_percentage).unwrap_or(0);
    let class = class_for_percentage(percentage).to_string();
    let tooltip = if let Some(template) = tooltip_format {
        active
            .map(|entry| format_entry(template, entry))
            .unwrap_or_else(|| "No Codex profiles found".to_string())
    } else {
        format_tooltip(&entries)
    };
    let alt = format_alt(&entries);

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
        entries.push(ProfileUsage {
            provider: "codex",
            name: current_profile,
            email,
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
        entries.push(ProfileUsage {
            provider: "codex",
            name,
            email,
            is_live: false,
            usage,
        });
    }

    if let Some(pi_auth) = live_pi.as_ref() {
        let usage = fetch_pi_rate_limit(ctx, pi_auth.clone()).map(|(usage, _)| usage);
        entries.push(ProfileUsage {
            provider: "pi",
            name: detect_pi_profile(ctx, pi_auth),
            email: extract_email_from_token(&pi_auth.access).unwrap_or_else(|| "?".to_string()),
            is_live: true,
            usage,
        });
    }

    for path in list_pi_profiles(ctx) {
        let Some(pi_auth) = read_pi_auth(&path).and_then(|auth| auth.openai_codex) else {
            continue;
        };
        if live_pi.as_ref() == Some(&pi_auth) {
            continue;
        }
        let name = profile_name(&path);
        let usage = fetch_pi_rate_limit_for_path(&path, pi_auth.clone()).map(|(usage, _)| usage);
        entries.push(ProfileUsage {
            provider: "pi",
            name,
            email: extract_email_from_token(&pi_auth.access).unwrap_or_else(|| "?".to_string()),
            is_live: false,
            usage,
        });
    }

    entries
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
    entry
        .usage
        .as_ref()
        .ok()?
        .rate_limit
        .primary_window
        .used_percent
        .map(|value| value.clamp(0.0, 100.0).round() as u8)
}

fn class_for_percentage(percentage: u8) -> &'static str {
    match percentage {
        90..=100 => "critical",
        80..=89 => "warning",
        _ => "ok",
    }
}

fn format_entry(template: &str, entry: &ProfileUsage) -> String {
    let mut values = FormatValues::default();
    if let Ok(usage) = &entry.usage {
        if let Some((used, reset_in, _)) = summarize_window(&usage.rate_limit.primary_window) {
            values.five_hour_pct = used;
            values.five_hour_reset = reset_in;
        }
        if let Some((used, reset_in, _)) = summarize_window(&usage.rate_limit.secondary_window) {
            values.seven_day_pct = used;
            values.seven_day_reset = reset_in;
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
    values.window = "5h".to_string();
    values.percent = values.five_hour_pct.clone();
    values.reset = values.five_hour_reset.clone();
    values.apply(template)
}

#[derive(Default)]
struct FormatValues {
    five_hour_pct: String,
    seven_day_pct: String,
    five_hour_reset: String,
    seven_day_reset: String,
    status: String,
    profile: String,
    provider: String,
    email: String,
    window: String,
    percent: String,
    reset: String,
}

impl FormatValues {
    fn apply(&self, template: &str) -> String {
        template
            .replace("{icon}", ICON)
            .replace("{icon_plain}", ICON)
            .replace("{time_icon}", TIME_ICON)
            .replace("{time_icon_plain}", TIME_ICON)
            .replace("{5h_pct}", value_or_unknown(&self.five_hour_pct))
            .replace("{7d_pct}", value_or_unknown(&self.seven_day_pct))
            .replace("{5h_reset}", value_or_unknown(&self.five_hour_reset))
            .replace("{7d_reset}", value_or_unknown(&self.seven_day_reset))
            .replace("{status}", &self.status)
            .replace("{profile}", &self.profile)
            .replace("{provider}", &self.provider)
            .replace("{email}", &self.email)
            .replace("{win}", &self.window)
            .replace("{pct}", value_or_unknown(&self.percent))
            .replace("{reset}", value_or_unknown(&self.reset))
    }
}

fn value_or_unknown(value: &str) -> &str {
    if value.is_empty() {
        "?"
    } else {
        value
    }
}

fn format_tooltip(entries: &[ProfileUsage]) -> String {
    if entries.is_empty() {
        return "No Codex profiles found".to_string();
    }

    let mut lines = vec!["Codex quotas".to_string()];
    for entry in entries {
        let marker = if entry.is_live { "*" } else { "-" };
        match &entry.usage {
            Ok(_) => lines.push(format!(
                "{} {} {}: 5h {}% reset {} | 7d {}% reset {}",
                marker,
                entry.provider,
                entry.name,
                format_entry("{5h_pct}", entry),
                format_entry("{5h_reset}", entry),
                format_entry("{7d_pct}", entry),
                format_entry("{7d_reset}", entry),
            )),
            Err(err) => lines.push(format!(
                "{} {} {}: unavailable ({})",
                marker, entry.provider, entry.name, err
            )),
        }
    }
    lines.join("\n")
}

fn format_alt(entries: &[ProfileUsage]) -> String {
    entries
        .iter()
        .map(|entry| {
            format!(
                "{}:{}:{}:{}:{}",
                entry.provider,
                entry.name,
                format_entry("{5h_pct}", entry),
                format_entry("{5h_reset}", entry),
                format_entry("{7d_pct}", entry)
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::{class_for_percentage, FormatValues};

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
        };

        assert_eq!(
            values.apply("{provider} {profile} {5h_pct}% {5h_reset} {7d_pct}% {7d_reset}"),
            "codex me 42% 1h 2m 12% 4d 3h 2m"
        );
    }

    #[test]
    fn waybar_class_reflects_active_usage() {
        assert_eq!(class_for_percentage(79), "ok");
        assert_eq!(class_for_percentage(80), "warning");
        assert_eq!(class_for_percentage(90), "critical");
    }
}

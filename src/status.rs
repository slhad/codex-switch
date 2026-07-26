use crate::data::{read_auth, read_pi_auth, Context, PiOpenAiCodexAuth};
use crate::jwt::{decode_token_payload, extract_email, extract_email_from_token};
use crate::profile::{detect_current_profile, list_pi_profiles, list_profiles, profile_name};
use crate::rate_limit::{
    fetch_pi_rate_limit, fetch_pi_rate_limit_for_path, fetch_rate_limit,
    fetch_rate_limit_for_auth_path, format_credit_amount, parse_reset_at, summarize_reset,
    summarize_window,
};
use crate::tracker::{
    fingerprint_secret, load_tracker, save_tracker, update_monthly_usage, update_rate_limit,
    upsert_session,
};
use chrono::{Local, TimeZone};

/// Truncate account ID to 8 chars with "..." if longer.
fn short_id(id: Option<&str>) -> String {
    match id {
        None => "?".to_string(),
        Some(s) if s.is_empty() => "?".to_string(),
        Some(s) if s.len() <= 8 => s.to_string(),
        Some(s) => format!("{}...", &s[..8]),
    }
}

fn auth_access_exp(auth: &crate::data::AuthFile) -> Option<u64> {
    auth.tokens
        .access_token
        .as_deref()
        .and_then(|token| decode_token_payload(token))
        .and_then(|payload| payload.exp)
        .map(|exp| exp * 1000)
}

fn format_epoch_ms(epoch_ms: Option<u64>) -> String {
    let Some(epoch_ms) = epoch_ms else {
        return "?".to_string();
    };

    let epoch_secs = (epoch_ms / 1000) as i64;
    match Local.timestamp_opt(epoch_secs, 0).single() {
        Some(timestamp) => timestamp.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
        None => "?".to_string(),
    }
}

fn update_entry_rate_limit_from_usage(
    entry: &mut crate::data::TrackedSession,
    usage: &crate::data::UsageResponse,
) {
    if let Some(monthly) = usage.monthly_limit() {
        update_monthly_usage(
            entry,
            Some(chrono::Utc::now().to_rfc3339()),
            monthly.limit.as_ref().and_then(|value| value.as_f64()),
            monthly.used.as_ref().and_then(|value| value.as_f64()),
            monthly.remaining.as_ref().and_then(|value| value.as_f64()),
            monthly.used_percent,
            monthly.remaining_percent,
            parse_reset_at(monthly.reset_at.as_ref()),
            usage.spend_control.as_ref().and_then(|value| value.reached),
            usage.plan_type.clone(),
        );
        return;
    }

    let primary = usage.five_hour_window();
    let secondary = usage.weekly_window();
    if primary.is_some() || secondary.is_some() {
        let resets_at = primary
            .and_then(|window| parse_reset_at(window.reset_at.as_ref()))
            .unwrap_or(0);
        let secondary_resets_at =
            secondary.and_then(|window| parse_reset_at(window.reset_at.as_ref()));
        update_rate_limit(
            entry,
            Some(chrono::Utc::now().to_rfc3339()),
            primary.and_then(|window| window.used_percent),
            resets_at,
            secondary.and_then(|window| window.used_percent),
            secondary_resets_at,
            usage.plan_type.clone(),
        );
    }
}

fn email_column_for_profile<'a>(profile_name: &str, email: &'a str) -> Option<&'a str> {
    if profile_name.eq_ignore_ascii_case(email) {
        None
    } else {
        Some(email)
    }
}

fn tracked_session_display_name(entry: &crate::data::TrackedSession) -> String {
    entry
        .profile
        .as_deref()
        .filter(|profile| !profile.is_empty())
        .or(entry.email.as_deref().filter(|email| !email.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| short_id(Some(entry.account_id.as_str())))
}

fn reset_is_active(reset_at: u64, now: u64) -> bool {
    reset_at > now
}

fn active_five_hour_window(
    usage: &crate::data::UsageResponse,
) -> Option<&crate::data::UsageWindow> {
    let window = usage.five_hour_window()?;
    let reset_at = parse_reset_at(window.reset_at.as_ref())?;
    reset_is_active(reset_at, chrono::Utc::now().timestamp() as u64).then_some(window)
}

fn format_percentage(value: Option<f64>) -> String {
    value
        .map(|value| format!("{}", value))
        .unwrap_or_else(|| "?".to_string())
}

fn print_monthly_usage(usage: &crate::data::UsageResponse) {
    let Some(monthly) = usage.monthly_limit() else {
        return;
    };

    println!(
        "  monthly used: {} / {} credits ({}%)",
        format_credit_amount(monthly.used.as_ref()),
        format_credit_amount(monthly.limit.as_ref()),
        format_percentage(monthly.used_percent)
    );
    println!(
        "  monthly remaining: {} credits ({}%)",
        format_credit_amount(monthly.remaining.as_ref()),
        format_percentage(monthly.remaining_percent)
    );
    if let Some((reset_in, reset_at)) = summarize_reset(monthly.reset_at.as_ref()) {
        println!("  monthly reset: in {} ({})", reset_in, reset_at);
    }
    if let Some(reached) = usage.spend_control.as_ref().and_then(|value| value.reached) {
        println!(
            "  monthly limit reached: {}",
            if reached { "yes" } else { "no" }
        );
    }
}

pub fn show_status(ctx: &Context, debug_usage: bool, debug_pi_usage: bool) {
    let live = ctx.live_auth.exists().then(|| read_auth(&ctx.live_auth));
    let current_profile = live.as_ref().and_then(|_| detect_current_profile(ctx));
    let live_email = live
        .as_ref()
        .and_then(extract_email)
        .unwrap_or_else(|| "?".to_string());
    let live_refresh = live
        .as_ref()
        .and_then(|auth| auth.last_refresh.as_deref())
        .unwrap_or("?");
    let live_mode = live
        .as_ref()
        .and_then(|auth| auth.auth_mode.as_deref())
        .unwrap_or("?");
    let latest_rate_limit = live.as_ref().map(|_| fetch_rate_limit(ctx));
    let mut tracker = load_tracker(ctx);

    if let Some((live, account_id)) = live.as_ref().and_then(|auth| {
        auth.tokens
            .account_id
            .as_deref()
            .map(|account_id| (auth, account_id))
    }) {
        let entry = upsert_session(
            &mut tracker,
            "codex:live",
            Some("codex".to_string()),
            Some(ctx.live_auth.display().to_string()),
            account_id,
            current_profile.clone(),
            Some(live_email.clone()),
            live.auth_mode.clone(),
            live.last_refresh.clone(),
            auth_access_exp(&live),
            live.tokens.refresh_token.is_some(),
            fingerprint_secret(live.tokens.refresh_token.as_deref()),
        );

        if let Some(Ok(usage)) = latest_rate_limit.as_ref() {
            update_entry_rate_limit_from_usage(entry, usage);
        }
    }

    println!("Current auth:");
    if let Some(live) = live.as_ref() {
        if let Some(ref p) = current_profile {
            println!("  profile: {}", p);
            if let Some(email) = email_column_for_profile(p, &live_email) {
                println!("  email: {}", email);
            }
        } else {
            println!("  profile: unknown");
            println!("  email: {}", live_email);
        }
        println!("  account: {}", short_id(live.tokens.account_id.as_deref()));
        println!("  mode: {}", live_mode);
        println!("  last_refresh: {}", live_refresh);
    } else {
        println!("  profile: missing");
        println!("  path: {}", ctx.live_auth.display());
    }

    if let Some(Ok(usage)) = &latest_rate_limit {
        if let Some((used, reset_in, reset_at)) =
            active_five_hour_window(usage).and_then(summarize_window)
        {
            println!("  5h used: {}%", used);
            println!("  5h reset: in {} ({})", reset_in, reset_at);
        }
        if let Some((used, reset_in, reset_at)) = usage.weekly_window().and_then(summarize_window) {
            println!("  7d used: {}%", used);
            println!("  7d reset: in {} ({})", reset_in, reset_at);
        }
        print_monthly_usage(usage);
    } else if let Some(entry) =
        live.as_ref()
            .and_then(|auth| auth.tokens.account_id.as_deref())
            .and_then(|account_id| {
                tracker.sessions.iter().find(|entry| {
                    entry.session_id == "codex:live" || entry.account_id == account_id
                })
            })
    {
        if let Some(rate_limit) = &entry.rate_limit {
            let now = chrono::Utc::now().timestamp() as u64;
            if reset_is_active(rate_limit.resets_at, now) {
                let used = format_percentage(rate_limit.used_percent);
                let reset_in = crate::rate_limit::format_duration_until(rate_limit.resets_at);
                let reset_at = Local
                    .timestamp_opt(rate_limit.resets_at as i64, 0)
                    .single()
                    .map(|ts| ts.format("%Y-%m-%d %H:%M:%S %Z").to_string())
                    .unwrap_or_else(|| "?".to_string());
                println!("  5h used: {}%", used);
                println!("  5h reset: in {} ({})", reset_in, reset_at);
            }
            if let Some(reset_at) = rate_limit
                .secondary_resets_at
                .filter(|reset_at| reset_is_active(*reset_at, now))
            {
                println!(
                    "  7d used: {}%",
                    format_percentage(rate_limit.secondary_used_percent)
                );
                println!(
                    "  7d reset: in {}",
                    crate::rate_limit::format_duration_until(reset_at)
                );
            }
        }
    } else if let Some(Err(err)) = &latest_rate_limit {
        println!("  usage: unavailable ({})", err);
    }

    if debug_usage {
        println!();
        println!("Raw usage payload:");
        match &latest_rate_limit {
            Some(Ok(usage)) => match serde_json::to_string_pretty(usage) {
                Ok(json) => println!("{}", json),
                Err(err) => println!("  failed to serialize usage payload: {}", err),
            },
            Some(Err(err)) => println!("  unavailable: {}", err),
            None => println!("  unavailable: missing live auth"),
        }
    }

    if let Some(pi_auth) = read_pi_auth(&ctx.pi_auth).and_then(|auth| auth.openai_codex) {
        show_pi_status(ctx, &pi_auth, &mut tracker, debug_pi_usage);
    }

    println!();
    println!("Saved Codex profiles:");

    let profiles = list_profiles(ctx);
    let profile_options =
        crate::profile_options::load(ctx).unwrap_or_else(|error| crate::data::die(&error));
    for p in &profiles {
        let name = profile_name(p);
        let marker = if current_profile.as_deref() == Some(&name) {
            '*'
        } else {
            '-'
        };

        // Check if this profile file is identical to the live auth
        let is_live_profile = std::fs::read(p).ok() == std::fs::read(&ctx.live_auth).ok();
        let (auth, profile_usage) = if is_live_profile {
            (read_auth(p), None)
        } else {
            match fetch_rate_limit_for_auth_path(p) {
                Ok((usage, auth)) => (auth, Some(usage)),
                Err(_) => (read_auth(p), None),
            }
        };
        let email = extract_email(&auth).unwrap_or_else(|| "?".to_string());
        let account = short_id(auth.tokens.account_id.as_deref());
        let refresh = auth.last_refresh.as_deref().unwrap_or("?");
        let mode = auth.auth_mode.as_deref().unwrap_or("?");

        if let Some(account_id) = auth.tokens.account_id.as_deref() {
            let entry = upsert_session(
                &mut tracker,
                &format!("codex:profile:{}", name),
                Some("codex".to_string()),
                Some(p.display().to_string()),
                account_id,
                Some(name.clone()),
                Some(email.clone()),
                auth.auth_mode.clone(),
                auth.last_refresh.clone(),
                auth_access_exp(&auth),
                auth.tokens.refresh_token.is_some(),
                fingerprint_secret(auth.tokens.refresh_token.as_deref()),
            );
            if let Some(usage) = profile_usage.as_ref() {
                update_entry_rate_limit_from_usage(entry, usage);
            }
        }

        let is_live = if is_live_profile { " [live]" } else { "" };
        let transfer = profile_options
            .profiles
            .get(&name)
            .and_then(|option| option.transfer.as_ref())
            .filter(|transfer| transfer.enabled)
            .map(|transfer| format!(" [transfer→pi:{}]", transfer.pi_profile))
            .unwrap_or_default();

        if let Some(email) = email_column_for_profile(&name, &email) {
            println!(
                "{} {:>8} {}  {}  {}  {}{}{}",
                marker, name, email, account, mode, refresh, is_live, transfer
            );
        } else {
            println!(
                "{} {:>8} {}  {}  {}{}{}",
                marker, name, account, mode, refresh, is_live, transfer
            );
        }
    }

    println!();
    println!("Saved PI profiles:");

    let pi_profiles = list_pi_profiles(ctx);
    for p in &pi_profiles {
        let name = profile_name(p);
        let marker = if detect_profile_from_pi_auth_file(ctx).as_deref() == Some(&name) {
            '*'
        } else {
            '-'
        };

        let pi_auth = read_pi_auth(p)
            .and_then(|auth| auth.openai_codex)
            .unwrap_or_else(|| {
                crate::data::die(&format!("invalid PI auth profile JSON: {}", p.display()))
            });
        let is_live_profile = read_pi_auth(&ctx.pi_auth)
            .and_then(|auth| auth.openai_codex)
            .as_ref()
            == Some(&pi_auth);
        let (pi_auth, profile_usage) = if is_live_profile {
            (pi_auth, None)
        } else {
            match fetch_pi_rate_limit_for_path(p, pi_auth.clone()) {
                Ok((usage, auth)) => (auth, Some(usage)),
                Err(_) => (pi_auth, None),
            }
        };
        let payload = decode_token_payload(&pi_auth.access);
        let email = payload
            .as_ref()
            .and_then(|payload| {
                payload.email.clone().or_else(|| {
                    payload
                        .openai_profile
                        .as_ref()
                        .and_then(|profile| profile.email.clone())
                })
            })
            .unwrap_or_else(|| "?".to_string());
        let account = short_id(pi_auth.account_id.as_deref());
        let expires = format_pi_expiry(pi_auth.expires);
        if let Some(account_id) = pi_auth.account_id.as_deref() {
            let entry = upsert_session(
                &mut tracker,
                &format!("pi:profile:{}", name),
                Some("pi".to_string()),
                Some(format!("{}#openai-codex", p.display())),
                account_id,
                Some(name.clone()),
                Some(email.clone()),
                Some("openai-codex".to_string()),
                None,
                pi_auth.expires,
                pi_auth.refresh.is_some(),
                fingerprint_secret(pi_auth.refresh.as_deref()),
            );
            if let Some(usage) = profile_usage.as_ref() {
                update_entry_rate_limit_from_usage(entry, usage);
            }
        }
        let is_live = if is_live_profile { " [live]" } else { "" };

        if let Some(email) = email_column_for_profile(&name, &email) {
            println!(
                "{} {:>8} {}  {}  {}{}",
                marker, name, email, account, expires, is_live
            );
        } else {
            println!("{} {:>8} {}  {}{}", marker, name, account, expires, is_live);
        }
    }

    crate::auto_switch::print_profile_options(ctx).unwrap_or_else(|error| crate::data::die(&error));
    crate::systemd::print_installation_status().unwrap_or_else(|error| crate::data::die(&error));

    save_tracker(ctx, &tracker);

    show_tracked_sessions(&tracker, Some("codex:live"));
}

fn show_pi_status(
    ctx: &Context,
    pi_auth: &PiOpenAiCodexAuth,
    tracker: &mut crate::data::AccountTracker,
    debug_pi_usage: bool,
) {
    let (pi_auth, pi_usage) = match fetch_pi_rate_limit(ctx, pi_auth.clone()) {
        Ok((usage, refreshed_auth)) => (refreshed_auth, Ok(usage)),
        Err(err) => (pi_auth.clone(), Err(err)),
    };
    let pi_profile = detect_profile_from_pi_auth(ctx, &pi_auth);
    let pi_payload = decode_token_payload(&pi_auth.access);
    let pi_email = pi_payload
        .as_ref()
        .and_then(|payload| {
            payload.email.clone().or_else(|| {
                payload
                    .openai_profile
                    .as_ref()
                    .and_then(|profile| profile.email.clone())
            })
        })
        .unwrap_or_else(|| "?".to_string());

    if let Some(account_id) = pi_auth.account_id.as_deref() {
        let entry = upsert_session(
            tracker,
            "pi:openai-codex",
            Some("pi".to_string()),
            Some(format!("{}#openai-codex", ctx.pi_auth.display())),
            account_id,
            Some(pi_profile.clone()),
            Some(pi_email.clone()),
            Some("openai-codex".to_string()),
            None,
            pi_auth.expires,
            pi_auth.refresh.is_some(),
            fingerprint_secret(pi_auth.refresh.as_deref()),
        );

        if let Ok(usage) = pi_usage.as_ref() {
            update_entry_rate_limit_from_usage(entry, usage);
        }
    }

    println!();
    println!("PI agent auth:");
    println!("  profile: {}", pi_profile);
    if let Some(email) = email_column_for_profile(&pi_profile, &pi_email) {
        println!("  email: {}", email);
    }
    println!("  account: {}", short_id(pi_auth.account_id.as_deref()));
    println!("  expires: {}", format_pi_expiry(pi_auth.expires));

    if let Some(payload) = pi_payload.as_ref() {
        let verified = payload
            .openai_profile
            .as_ref()
            .and_then(|profile| profile.email_verified)
            .map(|value| if value { "yes" } else { "no" })
            .unwrap_or("?");
        let token_plan = payload
            .openai_auth
            .as_ref()
            .and_then(|auth| auth.chatgpt_plan_type.as_deref())
            .unwrap_or("?");
        let token_account = payload
            .openai_auth
            .as_ref()
            .and_then(|auth| auth.chatgpt_account_id.as_deref())
            .unwrap_or("?");
        let token_account_user = payload
            .openai_auth
            .as_ref()
            .and_then(|auth| auth.chatgpt_account_user_id.as_deref())
            .unwrap_or("?");
        let user_id = payload
            .openai_auth
            .as_ref()
            .and_then(|auth| auth.chatgpt_user_id.as_deref())
            .or_else(|| payload.sub.as_deref())
            .unwrap_or("?");
        let client_id = payload.client_id.as_deref().unwrap_or("?");
        let session_id = payload.session_id.as_deref().unwrap_or("?");
        let scopes = payload
            .scp
            .as_ref()
            .map(|items| items.join(","))
            .unwrap_or_else(|| "?".to_string());

        println!("  email_verified: {}", verified);
        println!("  token_plan: {}", token_plan);
        println!("  token_account: {}", short_id(Some(token_account)));
        println!(
            "  token_account_user: {}",
            short_id(Some(token_account_user))
        );
        println!("  user_id: {}", short_id(Some(user_id)));
        println!("  client_id: {}", short_id(Some(client_id)));
        println!("  session_id: {}", short_id(Some(session_id)));
        println!("  token_iat: {}", format_epoch(payload.iat));
        println!("  token_exp: {}", format_epoch(payload.exp));
        println!("  scopes: {}", scopes);
    }

    match &pi_usage {
        Ok(usage) => {
            println!(
                "  usage_plan: {}",
                usage.plan_type.as_deref().unwrap_or("?")
            );
            if let Some((used, reset_in, reset_at)) =
                active_five_hour_window(usage).and_then(summarize_window)
            {
                println!("  5h used: {}%", used);
                println!("  5h reset: in {} ({})", reset_in, reset_at);
            }
            if let Some((used, reset_in, reset_at)) =
                usage.weekly_window().and_then(summarize_window)
            {
                println!("  7d used: {}%", used);
                println!("  7d reset: in {} ({})", reset_in, reset_at);
            }
            print_monthly_usage(usage);
        }
        Err(err) => println!("  usage: unavailable ({})", err),
    }

    if debug_pi_usage {
        println!();
        println!("Raw PI usage payload:");
        match &pi_usage {
            Ok(usage) => match serde_json::to_string_pretty(usage) {
                Ok(json) => println!("{}", json),
                Err(err) => println!("  failed to serialize PI usage payload: {}", err),
            },
            Err(err) => println!("  unavailable: {}", err),
        }
    }
}

fn detect_profile_from_pi_auth(ctx: &Context, pi_auth: &PiOpenAiCodexAuth) -> String {
    let pi_email = extract_email_from_token(&pi_auth.access);

    for path in list_pi_profiles(ctx) {
        let Some(profile_pi_auth) = read_pi_auth(&path).and_then(|auth| auth.openai_codex) else {
            continue;
        };
        if pi_auth.account_id.is_some() && profile_pi_auth.account_id == pi_auth.account_id {
            return profile_name(&path);
        }

        let profile_email = extract_email_from_token(&profile_pi_auth.access);
        if pi_email.is_some() && profile_email == pi_email {
            return profile_name(&path);
        }
    }

    for path in list_profiles(ctx) {
        let auth = read_auth(&path);
        if pi_auth.account_id.is_some() && auth.tokens.account_id == pi_auth.account_id {
            return profile_name(&path);
        }

        let profile_email = extract_email(&auth);
        if pi_email.is_some() && profile_email == pi_email {
            return profile_name(&path);
        }
    }

    "unknown".to_string()
}

fn detect_profile_from_pi_auth_file(ctx: &Context) -> Option<String> {
    let pi_auth = read_pi_auth(&ctx.pi_auth)?.openai_codex?;
    let profile = detect_profile_from_pi_auth(ctx, &pi_auth);
    (profile != "unknown").then_some(profile)
}

fn format_pi_expiry(expires_ms: Option<u64>) -> String {
    let Some(expires_ms) = expires_ms else {
        return "?".to_string();
    };

    let expires_secs = (expires_ms / 1000) as i64;
    match Local.timestamp_opt(expires_secs, 0).single() {
        Some(timestamp) => timestamp.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
        None => "?".to_string(),
    }
}

fn format_epoch(epoch_secs: Option<u64>) -> String {
    let Some(epoch_secs) = epoch_secs else {
        return "?".to_string();
    };

    match Local.timestamp_opt(epoch_secs as i64, 0).single() {
        Some(timestamp) => timestamp.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
        None => "?".to_string(),
    }
}

fn show_tracked_sessions(tracker: &crate::data::AccountTracker, current_session_id: Option<&str>) {
    if tracker.sessions.is_empty() {
        return;
    }

    println!();
    println!("Tracked sessions:");

    let live_sessions: Vec<_> = tracker
        .sessions
        .iter()
        .filter(|entry| !entry.session_id.contains(":profile:"))
        .collect();

    let mut sessions = tracker.sessions.clone();
    sessions.sort_by(|a, b| a.session_id.cmp(&b.session_id));

    for entry in sessions {
        if entry.session_id.contains(":profile:")
            && live_sessions.iter().any(|current| {
                entry.provider == current.provider
                    && entry.account_id == current.account_id
                    && entry.profile == current.profile
            })
        {
            continue;
        }

        let marker = if current_session_id == Some(entry.session_id.as_str()) {
            '*'
        } else {
            '-'
        };
        let provider = entry.provider.as_deref().unwrap_or("?");
        print!(
            "{} {:>5} {}  id:{}",
            marker,
            provider,
            tracked_session_display_name(&entry),
            entry.session_id
        );

        if entry.has_refresh {
            print!("  refresh:yes");
        } else {
            print!("  refresh:no");
        }

        let access_expires = format_epoch_ms(entry.access_expires_at);
        print!("  expires:{}", access_expires);

        if let Some(monthly) = entry.monthly_usage.as_ref() {
            let used = monthly
                .used
                .map(|value| format!("{:.2}", value))
                .unwrap_or_else(|| "?".to_string());
            let remaining = monthly
                .remaining
                .map(|value| format!("{:.2}", value))
                .unwrap_or_else(|| "?".to_string());
            let reset = monthly
                .resets_at
                .map(crate::rate_limit::format_duration_until)
                .unwrap_or_else(|| "?".to_string());
            print!(
                "  month:{}% used:{} remaining:{} reset:{}",
                format_percentage(monthly.used_percent),
                used,
                remaining,
                reset
            );
        } else if let Some(rate_limit) = entry.rate_limit.as_ref() {
            let now = chrono::Utc::now().timestamp() as u64;
            if reset_is_active(rate_limit.resets_at, now) {
                print!(
                    "  5h:{}% reset:{}",
                    format_percentage(rate_limit.used_percent),
                    crate::rate_limit::format_duration_until(rate_limit.resets_at)
                );
            }
            if let Some(reset_at) = rate_limit
                .secondary_resets_at
                .filter(|reset_at| reset_is_active(*reset_at, now))
            {
                print!(
                    "  7d:{}% reset:{}",
                    format_percentage(rate_limit.secondary_used_percent),
                    crate::rate_limit::format_duration_until(reset_at)
                );
            }
        }

        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        active_five_hour_window, email_column_for_profile, reset_is_active,
        tracked_session_display_name,
    };
    use crate::data::{TrackedSession, UsageResponse};

    #[test]
    fn hides_expired_five_hour_windows() {
        assert!(!reset_is_active(100, 100));
        assert!(!reset_is_active(99, 100));
        assert!(reset_is_active(101, 100));

        let now = chrono::Utc::now().timestamp() as u64;
        let expired: UsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 80,
                    "limit_window_seconds": 18000,
                    "reset_at": now.saturating_sub(1)
                }
            }
        }))
        .unwrap();
        let active: UsageResponse = serde_json::from_value(serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 80,
                    "limit_window_seconds": 18000,
                    "reset_at": now + 3600
                }
            }
        }))
        .unwrap();
        assert!(active_five_hour_window(&expired).is_none());
        assert!(active_five_hour_window(&active).is_some());
    }

    #[test]
    fn email_column_omits_email_when_profile_is_same_email() {
        assert_eq!(
            email_column_for_profile("user@example.com", "user@example.com"),
            None
        );
        assert_eq!(
            email_column_for_profile("USER@example.com", "user@example.com"),
            None
        );
    }

    #[test]
    fn email_column_keeps_email_when_profile_is_alias_or_unknown() {
        assert_eq!(
            email_column_for_profile("work", "user@example.com"),
            Some("user@example.com")
        );
        assert_eq!(email_column_for_profile("user@example.com", "?"), Some("?"));
    }

    #[test]
    fn tracked_session_display_prefers_profile_over_account() {
        let entry = TrackedSession {
            account_id: "acct_1234567890".to_string(),
            profile: Some("work".to_string()),
            email: Some("user@example.com".to_string()),
            ..TrackedSession::default()
        };

        assert_eq!(tracked_session_display_name(&entry), "work");
    }

    #[test]
    fn tracked_session_display_falls_back_when_profile_missing() {
        let email_entry = TrackedSession {
            account_id: "acct_1234567890".to_string(),
            email: Some("user@example.com".to_string()),
            ..TrackedSession::default()
        };
        let account_entry = TrackedSession {
            account_id: "acct_1234567890".to_string(),
            ..TrackedSession::default()
        };

        assert_eq!(
            tracked_session_display_name(&email_entry),
            "user@example.com"
        );
        assert_eq!(tracked_session_display_name(&account_entry), "acct_123...");
    }
}

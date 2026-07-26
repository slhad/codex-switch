use chrono::Utc;

use crate::data::{
    AccountTracker, Context, TrackedAuthSnapshot, TrackedMonthlyUsage, TrackedRateLimit,
    TrackedSession,
};

fn normalize_tracker(mut tracker: AccountTracker) -> AccountTracker {
    tracker
        .sessions
        .retain(|entry| !entry.session_id.is_empty());
    tracker
}

pub fn load_tracker(ctx: &Context) -> AccountTracker {
    let Ok(content) = std::fs::read_to_string(&ctx.tracker_file) else {
        return AccountTracker::default();
    };

    normalize_tracker(serde_json::from_str(&content).unwrap_or_default())
}

fn load_tracker_strict(ctx: &Context) -> Result<AccountTracker, String> {
    let content = match std::fs::read_to_string(&ctx.tracker_file) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AccountTracker::default())
        }
        Err(error) => {
            return Err(format!(
                "failed to read account tracker {}: {}",
                ctx.tracker_file.display(),
                error
            ))
        }
    };
    serde_json::from_str(&content)
        .map(normalize_tracker)
        .map_err(|error| {
            format!(
                "invalid account tracker JSON {}: {}",
                ctx.tracker_file.display(),
                error
            )
        })
}

fn save_tracker_result(ctx: &Context, tracker: &AccountTracker) -> Result<(), String> {
    std::fs::create_dir_all(&ctx.state_dir).map_err(|error| {
        format!(
            "failed to create state directory {}: {}",
            ctx.state_dir.display(),
            error
        )
    })?;

    let normalized = normalize_tracker(tracker.clone());
    let json = serde_json::to_string_pretty(&normalized)
        .map_err(|error| format!("failed to serialize account tracker: {}", error))?;
    let tmp_file = ctx.state_dir.join("accounts.json.tmp");
    std::fs::write(&tmp_file, json).map_err(|error| {
        format!(
            "failed to write temporary account tracker {}: {}",
            tmp_file.display(),
            error
        )
    })?;
    std::fs::rename(&tmp_file, &ctx.tracker_file).map_err(|error| {
        format!(
            "failed to replace account tracker {}: {}",
            ctx.tracker_file.display(),
            error
        )
    })
}

pub fn save_tracker(ctx: &Context, tracker: &AccountTracker) {
    let _ = save_tracker_result(ctx, tracker);
}

pub fn remove_session(ctx: &Context, session_id: &str) -> Result<bool, String> {
    if session_id.is_empty() {
        return Err("tracked session ID cannot be empty".to_string());
    }

    let mut tracker = load_tracker_strict(ctx)?;
    let original_len = tracker.sessions.len();
    tracker
        .sessions
        .retain(|entry| entry.session_id != session_id);
    let removed = tracker.sessions.len() != original_len;
    if !removed {
        return Ok(false);
    }

    if tracker
        .last_quota_hit
        .as_ref()
        .and_then(|hit| hit.session_id.as_deref())
        == Some(session_id)
    {
        tracker.last_quota_hit = None;
    }
    save_tracker_result(ctx, &tracker)?;
    Ok(true)
}

pub fn snapshot_auth_json(ctx: &Context, auth_json: String, profile: Option<String>) {
    let mut tracker = load_tracker(ctx);
    tracker.last_snapshot = Some(TrackedAuthSnapshot {
        auth_json,
        observed_at: Some(Utc::now().to_rfc3339()),
        profile,
    });
    save_tracker(ctx, &tracker);
}

pub fn snapshot_live_auth(ctx: &Context, profile: Option<String>) {
    let auth_json = std::fs::read_to_string(&ctx.live_auth).unwrap_or_else(|e| {
        crate::data::die(&format!(
            "failed to read live auth for tracker snapshot: {}",
            e
        ))
    });
    snapshot_auth_json(ctx, auth_json, profile);
}

pub fn load_last_snapshot(ctx: &Context) -> Option<TrackedAuthSnapshot> {
    load_tracker(ctx).last_snapshot
}

pub fn fingerprint_secret(secret: Option<&str>) -> Option<String> {
    let secret = secret?;
    if secret.is_empty() {
        return None;
    }

    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in secret.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    Some(format!("{:016x}", hash))
}

#[allow(clippy::too_many_arguments)]
pub fn upsert_session<'a>(
    tracker: &'a mut AccountTracker,
    session_id: &str,
    provider: Option<String>,
    auth_path: Option<String>,
    account_id: &str,
    profile: Option<String>,
    email: Option<String>,
    auth_mode: Option<String>,
    last_refresh: Option<String>,
    access_expires_at: Option<u64>,
    has_refresh: bool,
    refresh_fingerprint: Option<String>,
) -> &'a mut TrackedSession {
    let now = Utc::now().to_rfc3339();

    let idx = tracker
        .sessions
        .iter()
        .position(|entry| entry.session_id == session_id)
        .unwrap_or_else(|| {
            tracker.sessions.push(TrackedSession {
                session_id: session_id.to_string(),
                account_id: account_id.to_string(),
                ..TrackedSession::default()
            });
            tracker.sessions.len() - 1
        });

    let entry = &mut tracker.sessions[idx];
    entry.session_id = session_id.to_string();
    if provider.is_some() {
        entry.provider = provider;
    }
    if auth_path.is_some() {
        entry.auth_path = auth_path;
    }
    entry.account_id = account_id.to_string();
    if profile.is_some() {
        entry.profile = profile;
    }
    if email.is_some() {
        entry.email = email;
    }
    if auth_mode.is_some() {
        entry.auth_mode = auth_mode;
    }
    if last_refresh.is_some() {
        entry.last_refresh = last_refresh;
    }
    if access_expires_at.is_some() {
        entry.access_expires_at = access_expires_at;
    }
    entry.has_refresh = has_refresh;
    if refresh_fingerprint.is_some() {
        entry.refresh_fingerprint = refresh_fingerprint;
    }
    entry.last_seen_at = Some(now);
    entry
}

pub fn update_rate_limit(
    entry: &mut TrackedSession,
    observed_at: Option<String>,
    used_percent: Option<f64>,
    resets_at: u64,
    secondary_used_percent: Option<f64>,
    secondary_resets_at: Option<u64>,
    plan_type: Option<String>,
) {
    entry.rate_limit = Some(TrackedRateLimit {
        observed_at,
        used_percent,
        resets_at,
        secondary_used_percent,
        secondary_resets_at,
        plan_type,
    });
    entry.monthly_usage = None;
}

#[allow(clippy::too_many_arguments)]
pub fn update_monthly_usage(
    entry: &mut TrackedSession,
    observed_at: Option<String>,
    limit: Option<f64>,
    used: Option<f64>,
    remaining: Option<f64>,
    used_percent: Option<f64>,
    remaining_percent: Option<f64>,
    resets_at: Option<u64>,
    reached: Option<bool>,
    plan_type: Option<String>,
) {
    entry.monthly_usage = Some(TrackedMonthlyUsage {
        observed_at,
        limit,
        used,
        remaining,
        used_percent,
        remaining_percent,
        resets_at,
        reached,
        plan_type,
    });
    entry.rate_limit = None;
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        fingerprint_secret, load_last_snapshot, load_tracker, remove_session, save_tracker,
        save_tracker_result, snapshot_live_auth, update_monthly_usage, update_rate_limit,
        upsert_session,
    };
    use crate::data::{
        AccountTracker, Context, TrackedAuthSnapshot, TrackedQuotaHit, TrackedSession,
    };

    fn test_context(name: &str) -> (Context, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "codex-switch-{}-{}-{}",
            name,
            std::process::id(),
            unique
        ));
        let codex_dir = base.join(".codex");
        let state_dir = base.join(".local").join("state").join("codex-switch");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let ctx = Context {
            live_auth: codex_dir.join("auth.json"),
            pi_auth: base.join(".pi").join("agent").join("auth.json"),
            state_dir: state_dir.clone(),
            tracker_file: state_dir.join("accounts.json"),
        };
        (ctx, base)
    }

    #[test]
    fn snapshot_live_auth_persists_recovery_state() {
        let (ctx, base) = test_context("tracker-snapshot");
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"id","account_id":"acct-1"}}"#,
        )
        .unwrap();

        snapshot_live_auth(&ctx, Some("work".to_string()));

        let snapshot = load_last_snapshot(&ctx).unwrap();
        assert_eq!(snapshot.profile.as_deref(), Some("work"));
        assert!(snapshot.auth_json.contains(r#""account_id":"acct-1""#));

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn save_tracker_round_trips_last_snapshot() {
        let (ctx, base) = test_context("tracker-roundtrip");
        let mut tracker = AccountTracker::default();
        tracker.last_snapshot = Some(TrackedAuthSnapshot {
            auth_json: "{\"tokens\":{\"id_token\":\"id\"}}".to_string(),
            observed_at: Some("2026-01-01T00:00:00Z".to_string()),
            profile: Some("personal".to_string()),
        });

        save_tracker(&ctx, &tracker);

        let loaded = load_tracker(&ctx);
        let snapshot = loaded.last_snapshot.unwrap();
        assert_eq!(snapshot.profile.as_deref(), Some("personal"));
        assert_eq!(
            snapshot.observed_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn fingerprints_refresh_secret_without_storing_it() {
        let fingerprint = fingerprint_secret(Some("rt.example.refresh")).unwrap();
        assert_eq!(fingerprint.len(), 16);
        assert_ne!(fingerprint, "rt.example.refresh");
        assert_eq!(fingerprint_secret(None), None);
        assert_eq!(fingerprint_secret(Some("")), None);
    }

    #[test]
    fn upsert_session_reuses_existing_session_id() {
        let mut tracker = AccountTracker::default();
        upsert_session(
            &mut tracker,
            "codex:live",
            Some("codex".to_string()),
            Some("/tmp/auth.json".to_string()),
            "acct-1",
            Some("mate".to_string()),
            Some("mate@example.com".to_string()),
            Some("chatgpt".to_string()),
            Some("2026-06-16T00:00:00Z".to_string()),
            Some(123),
            true,
            Some("abcd".to_string()),
        );
        upsert_session(
            &mut tracker,
            "codex:live",
            Some("codex".to_string()),
            Some("/tmp/auth.json".to_string()),
            "acct-1",
            Some("mate".to_string()),
            Some("mate@example.com".to_string()),
            Some("chatgpt".to_string()),
            Some("2026-06-17T00:00:00Z".to_string()),
            Some(456),
            true,
            Some("efgh".to_string()),
        );

        assert_eq!(tracker.sessions.len(), 1);
        assert_eq!(tracker.sessions[0].access_expires_at, Some(456));
        assert_eq!(
            tracker.sessions[0].refresh_fingerprint.as_deref(),
            Some("efgh")
        );
    }

    #[test]
    fn tracker_normalizes_invalid_and_empty_sessions() {
        let (ctx, base) = test_context("tracker-normalize");
        assert!(load_tracker(&ctx).sessions.is_empty());
        std::fs::create_dir_all(&ctx.state_dir).unwrap();
        std::fs::write(&ctx.tracker_file, "invalid").unwrap();
        assert!(load_tracker(&ctx).sessions.is_empty());
        std::fs::write(
            &ctx.tracker_file,
            r#"{"sessions":[{"session_id":""},{"session_id":"kept"}]}"#,
        )
        .unwrap();
        assert_eq!(load_tracker(&ctx).sessions.len(), 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn upsert_preserves_optional_fields_and_updates_rate_limit() {
        let mut tracker = AccountTracker::default();
        let entry = upsert_session(
            &mut tracker,
            "pi:live",
            None,
            None,
            "acct",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        );
        update_rate_limit(entry, None, Some(9.0), 123, Some(22.0), Some(456), None);
        let rate = tracker.sessions[0].rate_limit.as_ref().unwrap();
        assert_eq!(rate.used_percent, Some(9.0));
        assert_eq!(rate.secondary_resets_at, Some(456));

        update_monthly_usage(
            &mut tracker.sessions[0],
            Some("2026-01-01T00:00:00Z".to_string()),
            Some(100.0),
            Some(25.0),
            Some(75.0),
            Some(25.0),
            Some(75.0),
            Some(123),
            Some(false),
            Some("business".to_string()),
        );
        let monthly = tracker.sessions[0].monthly_usage.as_ref().unwrap();
        assert_eq!(monthly.remaining, Some(75.0));
        assert!(tracker.sessions[0].rate_limit.is_none());
    }

    #[test]
    fn removes_exact_session_and_related_quota_hit() {
        let (ctx, base) = test_context("remove-session");
        let tracker = AccountTracker {
            sessions: vec![
                TrackedSession {
                    session_id: "pi:profile:me".to_string(),
                    ..TrackedSession::default()
                },
                TrackedSession {
                    session_id: "codex:live".to_string(),
                    ..TrackedSession::default()
                },
            ],
            last_quota_hit: Some(TrackedQuotaHit {
                session_id: Some("pi:profile:me".to_string()),
                ..TrackedQuotaHit::default()
            }),
            ..AccountTracker::default()
        };
        save_tracker(&ctx, &tracker);

        assert!(remove_session(&ctx, "pi:profile:me").unwrap());
        let loaded = load_tracker(&ctx);
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].session_id, "codex:live");
        assert!(loaded.last_quota_hit.is_none());
        assert!(!remove_session(&ctx, "pi:profile:missing").unwrap());
        assert!(remove_session(&ctx, "").is_err());

        std::fs::write(&ctx.tracker_file, "invalid").unwrap();
        assert!(remove_session(&ctx, "codex:live").is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn remove_session_handles_missing_and_unreadable_tracker_files() {
        let (ctx, base) = test_context("remove-session-read-errors");
        assert!(!remove_session(&ctx, "missing").unwrap());

        std::fs::create_dir_all(&ctx.tracker_file).unwrap();
        let error = remove_session(&ctx, "missing").unwrap_err();
        assert!(error.contains("failed to read account tracker"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn save_tracker_tolerates_unwritable_paths() {
        let (blocked_dir_ctx, blocked_dir_base) = test_context("tracker-blocked-dir");
        std::fs::create_dir_all(blocked_dir_ctx.state_dir.parent().unwrap()).unwrap();
        std::fs::write(&blocked_dir_ctx.state_dir, "not a directory").unwrap();
        save_tracker(&blocked_dir_ctx, &AccountTracker::default());
        assert!(!blocked_dir_ctx.tracker_file.exists());
        std::fs::remove_dir_all(blocked_dir_base).unwrap();

        let (blocked_tmp_ctx, blocked_tmp_base) = test_context("tracker-blocked-tmp");
        std::fs::create_dir_all(blocked_tmp_ctx.state_dir.join("accounts.json.tmp")).unwrap();
        save_tracker(&blocked_tmp_ctx, &AccountTracker::default());
        assert!(!blocked_tmp_ctx.tracker_file.exists());
        std::fs::remove_dir_all(blocked_tmp_base).unwrap();

        let (blocked_rename_ctx, blocked_rename_base) = test_context("tracker-blocked-rename");
        std::fs::create_dir_all(&blocked_rename_ctx.tracker_file).unwrap();
        assert!(
            save_tracker_result(&blocked_rename_ctx, &AccountTracker::default())
                .unwrap_err()
                .contains("failed to replace account tracker")
        );
        std::fs::remove_dir_all(blocked_rename_base).unwrap();
    }
}

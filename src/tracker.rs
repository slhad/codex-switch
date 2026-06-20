use chrono::Utc;

use crate::data::{AccountTracker, Context, TrackedAuthSnapshot, TrackedRateLimit, TrackedSession};

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

pub fn save_tracker(ctx: &Context, tracker: &AccountTracker) {
    if std::fs::create_dir_all(&ctx.state_dir).is_err() {
        return;
    }

    let normalized = normalize_tracker(tracker.clone());
    let Ok(json) = serde_json::to_string_pretty(&normalized) else {
        return;
    };
    let tmp_file = ctx.state_dir.join("accounts.json.tmp");
    if std::fs::write(&tmp_file, json).is_err() {
        return;
    }
    let _ = std::fs::rename(tmp_file, &ctx.tracker_file);
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
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        fingerprint_secret, load_last_snapshot, load_tracker, save_tracker, snapshot_live_auth,
        upsert_session,
    };
    use crate::data::{AccountTracker, Context, TrackedAuthSnapshot};

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
}

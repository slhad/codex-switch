use crate::data::{read_auth, Context};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProfileTransfers {
    #[serde(default)]
    pub codex_to_pi: BTreeMap<String, String>,
}

pub fn load_profile_transfers(ctx: &Context) -> ProfileTransfers {
    let path = ctx.profile_transfers_path();
    let Ok(content) = fs::read_to_string(&path) else {
        return ProfileTransfers::default();
    };
    match serde_json::from_str(&content) {
        Ok(transfers) => transfers,
        Err(error) => crate::data::die(&format!(
            "invalid profile transfer config {}: {}",
            path.display(),
            error
        )),
    }
}

pub fn remember_codex_to_pi_transfer(ctx: &Context, codex_name: &str, pi_name: &str) {
    let mut transfers = load_profile_transfers(ctx);
    transfers
        .codex_to_pi
        .insert(codex_name.to_string(), pi_name.to_string());
    let path = ctx.profile_transfers_path();
    if let Err(error) = fs::create_dir_all(&ctx.state_dir) {
        crate::data::die(&format!(
            "failed to create state directory {}: {}",
            ctx.state_dir.display(),
            error
        ));
    }
    let content = match serde_json::to_string_pretty(&transfers) {
        Ok(content) => content,
        Err(error) => {
            crate::data::die(&format!("failed to serialize profile transfers: {}", error))
        }
    };
    if let Err(error) = fs::write(&path, format!("{}\n", content)) {
        crate::data::die(&format!(
            "failed to save profile transfer config {}: {}",
            path.display(),
            error
        ));
    }
}

/// List all profile paths, filtering out `bak-*` and empty names.
pub fn list_profiles(ctx: &Context) -> Vec<std::path::PathBuf> {
    let mut entries = Vec::new();

    let entries_iter = match fs::read_dir(ctx.codex_profiles_dir()) {
        Ok(e) => e,
        Err(_) => return entries,
    };

    for entry in entries_iter.filter_map(|e| e.ok()) {
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        if let Some(suffix) = fname_str.strip_prefix("auth.json.") {
            // Skip backup files
            if suffix.starts_with("bak-") || suffix.is_empty() {
                continue;
            }
            entries.push(ctx.profile_path(suffix));
        }
    }

    entries.sort();
    entries
}

/// List all PI profile paths, filtering out `bak-*` and empty names.
pub fn list_pi_profiles(ctx: &Context) -> Vec<std::path::PathBuf> {
    let mut entries = Vec::new();

    let entries_iter = match fs::read_dir(ctx.pi_profiles_dir()) {
        Ok(e) => e,
        Err(_) => return entries,
    };

    for entry in entries_iter.filter_map(|e| e.ok()) {
        let fname = entry.file_name();
        let fname_str = fname.to_string_lossy();
        if let Some(suffix) = fname_str.strip_prefix("auth.json.") {
            if suffix.starts_with("bak-") || suffix.is_empty() {
                continue;
            }
            entries.push(ctx.pi_profile_path(suffix));
        }
    }

    entries.sort();
    entries
}

/// Extract the profile name from a profile path.
pub fn profile_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .strip_prefix("auth.json.")
        .unwrap_or("")
        .to_string()
}

/// Detect which profile matches the current live auth.json by account_id (primary) or email (fallback).
pub fn detect_current_profile(ctx: &Context) -> Option<String> {
    let live = read_auth(&ctx.live_auth);
    let live_account_id = &live.tokens.account_id;
    let live_email = crate::jwt::extract_email(&live);

    let profiles = list_profiles(ctx);

    // First pass: match by account_id
    for p in &profiles {
        let auth = read_auth(p);
        if let Some(ref live_id) = live_account_id {
            if let Some(ref profile_id) = auth.tokens.account_id {
                if profile_id == live_id {
                    return Some(profile_name(p));
                }
            }
        }
    }

    // Second pass: match by email
    for p in &profiles {
        let auth = read_auth(p);
        let profile_email = crate::jwt::extract_email(&auth);
        if let Some(ref live_e) = live_email {
            if let Some(ref p_e) = profile_email {
                if p_e == live_e {
                    return Some(profile_name(p));
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        detect_current_profile, list_pi_profiles, list_profiles, load_profile_transfers,
        profile_name, remember_codex_to_pi_transfer,
    };
    use crate::data::Context;
    use std::path::PathBuf;

    fn context(name: &str) -> (Context, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "codex-switch-profile-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let state_dir = base.join("state");
        (
            Context {
                live_auth: base.join("live.json"),
                pi_auth: base.join("pi.json"),
                tracker_file: state_dir.join("accounts.json"),
                state_dir,
            },
            base,
        )
    }

    fn auth(account: Option<&str>, token: &str) -> String {
        serde_json::json!({"tokens": {"id_token": token, "account_id": account}}).to_string()
    }

    #[test]
    fn lists_only_named_non_backup_profiles() {
        let (ctx, base) = context("list");
        assert!(list_profiles(&ctx).is_empty());
        assert!(list_pi_profiles(&ctx).is_empty());
        std::fs::create_dir_all(ctx.codex_profiles_dir()).unwrap();
        std::fs::create_dir_all(ctx.pi_profiles_dir()).unwrap();
        for name in ["auth.json.work", "auth.json.bak-old", "auth.json.", "other"] {
            std::fs::write(ctx.codex_profiles_dir().join(name), "{}").unwrap();
            std::fs::write(ctx.pi_profiles_dir().join(name), "{}").unwrap();
        }
        assert_eq!(list_profiles(&ctx), vec![ctx.profile_path("work")]);
        assert_eq!(list_pi_profiles(&ctx), vec![ctx.pi_profile_path("work")]);
        assert_eq!(profile_name(&ctx.profile_path("work")), "work");
        assert_eq!(profile_name(std::path::Path::new("other")), "");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn remembers_codex_to_pi_transfers_by_source_profile() {
        let (ctx, base) = context("transfers");
        assert!(load_profile_transfers(&ctx).codex_to_pi.is_empty());

        remember_codex_to_pi_transfer(&ctx, "mate", "mate");
        remember_codex_to_pi_transfer(&ctx, "work", "work-pi");

        let transfers = load_profile_transfers(&ctx);
        assert_eq!(
            transfers.codex_to_pi.get("mate").map(String::as_str),
            Some("mate")
        );
        assert_eq!(
            transfers.codex_to_pi.get("work").map(String::as_str),
            Some("work-pi")
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn detects_by_account_then_email_and_handles_no_match() {
        let (ctx, base) = context("detect");
        std::fs::create_dir_all(ctx.codex_profiles_dir()).unwrap();
        let email_token = "x.eyJlbWFpbCI6InNhbWVAZXhhbXBsZS5jb20ifQ.x";
        std::fs::write(&ctx.live_auth, auth(Some("acct-live"), email_token)).unwrap();
        std::fs::write(
            ctx.profile_path("account"),
            auth(Some("acct-live"), "x.e30.x"),
        )
        .unwrap();
        std::fs::write(ctx.profile_path("email"), auth(Some("other"), email_token)).unwrap();
        assert_eq!(detect_current_profile(&ctx).as_deref(), Some("account"));

        std::fs::remove_file(ctx.profile_path("account")).unwrap();
        assert_eq!(detect_current_profile(&ctx).as_deref(), Some("email"));

        let other_email_token = "x.eyJlbWFpbCI6Im90aGVyQGV4YW1wbGUuY29tIn0.x";
        std::fs::write(&ctx.live_auth, auth(None, other_email_token)).unwrap();
        assert!(detect_current_profile(&ctx).is_none());

        std::fs::write(&ctx.live_auth, auth(Some("unmatched"), email_token)).unwrap();
        std::fs::write(ctx.profile_path("email"), auth(None, "x.e30.x")).unwrap();
        assert!(detect_current_profile(&ctx).is_none());
        std::fs::remove_dir_all(base).unwrap();
    }
}

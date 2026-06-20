use crate::data::{read_auth, Context};
use std::fs;

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

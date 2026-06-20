use crate::data::{read_auth, read_pi_auth, Context, PiAuthFile, PiOpenAiCodexAuth};
use crate::jwt::{decode_token_payload, extract_email_from_token};
use crate::profile::{detect_current_profile, list_pi_profiles, profile_name};
use crate::status::show_status;
use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchScope {
    Both,
    CodexOnly,
    PiOnly,
}

fn write_file_atomically(path: &Path, content: &str) {
    let parent = path
        .parent()
        .unwrap_or_else(|| crate::data::die("live auth path has no parent directory"));
    let tmp_path = parent.join("auth.json.tmp");

    fs::write(&tmp_path, content)
        .unwrap_or_else(|e| crate::data::die(&format!("failed to write temp auth file: {}", e)));
    fs::rename(&tmp_path, path)
        .unwrap_or_else(|e| crate::data::die(&format!("failed to replace live auth: {}", e)));
}

fn restore_last_snapshot(ctx: &Context) -> crate::data::TrackedAuthSnapshot {
    let snapshot = crate::tracker::load_last_snapshot(ctx)
        .unwrap_or_else(|| crate::data::die("no tracker-backed auth snapshot found"));
    let _ = serde_json::from_str::<crate::data::AuthFile>(&snapshot.auth_json)
        .unwrap_or_else(|e| crate::data::die(&format!("invalid tracker snapshot JSON: {}", e)));

    if ctx.live_auth.exists() {
        let live_json = fs::read_to_string(&ctx.live_auth).unwrap_or_else(|e| {
            crate::data::die(&format!("failed to read live auth before restore: {}", e))
        });
        crate::tracker::snapshot_auth_json(ctx, live_json, None);
    }

    write_file_atomically(&ctx.live_auth, &snapshot.auth_json);
    snapshot
}

fn save_named_profile_state(ctx: &Context, name: &str) {
    save_codex_profile_if_present(ctx, name);
    save_pi_profile_if_present(ctx, name);
}

fn save_codex_profile(ctx: &Context, name: &str) {
    let codex_profile = ctx.profile_path(name);
    if let Some(parent) = codex_profile.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| {
            crate::data::die(&format!(
                "failed to create Codex profiles directory {}: {}",
                parent.display(),
                e
            ))
        });
    }
    fs::copy(&ctx.live_auth, &codex_profile).unwrap_or_else(|e| {
        crate::data::die(&format!(
            "failed to save current auth to profile {}: {}",
            name, e
        ))
    });
    println!("Saved current auth into profile: {}", name);
}

fn save_codex_profile_if_present(ctx: &Context, name: &str) {
    if !ctx.live_auth.exists() {
        return;
    }

    save_codex_profile(ctx, name);
}

fn load_pi_openai_codex(path: &Path) -> PiOpenAiCodexAuth {
    read_pi_auth(path)
        .and_then(|auth| auth.openai_codex)
        .unwrap_or_else(|| {
            crate::data::die(&format!("missing openai-codex entry: {}", path.display()))
        })
}

fn write_pi_openai_codex_profile(path: &Path, entry: &PiOpenAiCodexAuth) {
    let profile = PiAuthFile {
        openai_codex: Some(entry.clone()),
    };
    let content = serde_json::to_string_pretty(&profile)
        .unwrap_or_else(|e| crate::data::die(&format!("failed to serialize PI profile: {}", e)));
    fs::write(path, format!("{}\n", content)).unwrap_or_else(|e| {
        crate::data::die(&format!(
            "failed to write PI profile {}: {}",
            path.display(),
            e
        ))
    });
}

fn write_live_pi_openai_codex(ctx: &Context, entry: &PiOpenAiCodexAuth) {
    let mut root = if ctx.pi_auth.exists() {
        let content = fs::read_to_string(&ctx.pi_auth).unwrap_or_else(|e| {
            crate::data::die(&format!(
                "failed to read PI auth file {}: {}",
                ctx.pi_auth.display(),
                e
            ))
        });
        serde_json::from_str::<serde_json::Value>(&content).unwrap_or_else(|e| {
            crate::data::die(&format!(
                "invalid PI auth JSON {}: {}",
                ctx.pi_auth.display(),
                e
            ))
        })
    } else {
        serde_json::json!({})
    };

    let Some(object) = root.as_object_mut() else {
        crate::data::die(&format!(
            "PI auth file root is not a JSON object: {}",
            ctx.pi_auth.display()
        ));
    };
    object.insert(
        "openai-codex".to_string(),
        serde_json::to_value(entry).unwrap_or_else(|e| {
            crate::data::die(&format!("failed to serialize openai-codex entry: {}", e))
        }),
    );

    if let Some(parent) = ctx.pi_auth.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| {
            crate::data::die(&format!(
                "failed to create PI auth directory {}: {}",
                parent.display(),
                e
            ))
        });
    }

    let content = serde_json::to_string_pretty(&root)
        .unwrap_or_else(|e| crate::data::die(&format!("failed to serialize PI auth file: {}", e)));
    fs::write(&ctx.pi_auth, format!("{}\n", content)).unwrap_or_else(|e| {
        crate::data::die(&format!(
            "failed to write PI auth file {}: {}",
            ctx.pi_auth.display(),
            e
        ))
    });
}

fn auth_to_pi_openai_codex(auth: &crate::data::AuthFile) -> PiOpenAiCodexAuth {
    let access = auth
        .tokens
        .access_token
        .clone()
        .unwrap_or_else(|| crate::data::die("source Codex profile is missing access_token"));

    let expires = decode_token_payload(&access)
        .and_then(|payload| payload.exp)
        .map(|exp| exp * 1000);

    PiOpenAiCodexAuth {
        auth_type: Some("oauth".to_string()),
        access,
        refresh: auth.tokens.refresh_token.clone(),
        account_id: auth.tokens.account_id.clone(),
        expires,
    }
}

fn pi_profiles_match(a: &PiOpenAiCodexAuth, b: &PiOpenAiCodexAuth) -> bool {
    if a.account_id.is_some() && a.account_id == b.account_id {
        return true;
    }

    let a_email = extract_email_from_token(&a.access);
    let b_email = extract_email_from_token(&b.access);
    if a_email.is_some() && a_email == b_email {
        return true;
    }

    a.access == b.access || (a.refresh.is_some() && a.refresh == b.refresh)
}

fn existing_pi_profile_for_auth(ctx: &Context, entry: &PiOpenAiCodexAuth) -> Option<String> {
    for path in list_pi_profiles(ctx) {
        let name = profile_name(&path);
        let Ok(existing) = std::panic::catch_unwind(|| load_pi_openai_codex(&path)) else {
            continue;
        };
        if pi_profiles_match(entry, &existing) {
            return Some(name);
        }
    }

    None
}

fn save_pi_profile_if_present(ctx: &Context, name: &str) {
    if !ctx.pi_auth.exists() {
        return;
    }

    let openai_codex = load_pi_openai_codex(&ctx.pi_auth);
    if let Some(existing_name) = existing_pi_profile_for_auth(ctx, &openai_codex) {
        if existing_name != name {
            println!(
                "Current PI auth already matches profile {}; skipped saving as {}",
                existing_name, name
            );
            return;
        }
    }

    let pi_profile = ctx.pi_profile_path(name);
    if let Some(parent) = pi_profile.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| {
            crate::data::die(&format!(
                "failed to create PI profiles directory {}: {}",
                parent.display(),
                e
            ))
        });
    }
    write_pi_openai_codex_profile(&pi_profile, &openai_codex);
    println!("Saved current PI auth into profile: {}", name);
}

pub fn save_profile(ctx: &Context, store: &str, name: &str) {
    match store {
        "codex" => {
            if !ctx.live_auth.exists() {
                crate::data::die(&format!(
                    "missing live auth file: {}",
                    ctx.live_auth.display()
                ));
            }
            let _ = read_auth(&ctx.live_auth);
            save_codex_profile(ctx, name);
        }
        "pi" => {
            if !ctx.pi_auth.exists() {
                crate::data::die(&format!("missing PI auth file: {}", ctx.pi_auth.display()));
            }
            let _ = read_pi_auth(&ctx.pi_auth).unwrap_or_else(|| {
                crate::data::die(&format!("invalid PI auth JSON: {}", ctx.pi_auth.display()))
            });
            save_pi_profile_if_present(ctx, name);
        }
        _ => crate::data::die("save-profile STORE must be `codex` or `pi`"),
    }

    println!();
    show_status(ctx, false, false);
}

pub fn transfer_profile(
    ctx: &Context,
    source_store: &str,
    source_name: &str,
    target_store: &str,
    target_name: &str,
) {
    match (source_store, target_store) {
        ("codex", "codex") => {
            let source = ctx.profile_path(source_name);
            if !source.exists() {
                crate::data::die(&format!(
                    "missing source profile file: {}",
                    source.display()
                ));
            }
            let _ = read_auth(&source);
            let target = ctx.profile_path(target_name);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|e| {
                    crate::data::die(&format!(
                        "failed to create Codex profiles directory {}: {}",
                        parent.display(),
                        e
                    ))
                });
            }
            fs::copy(&source, &target).unwrap_or_else(|e| {
                crate::data::die(&format!(
                    "failed to transfer Codex profile {} -> {}: {}",
                    source_name, target_name, e
                ))
            });
        }
        ("pi", "pi") => {
            let source = ctx.pi_profile_path(source_name);
            if !source.exists() {
                crate::data::die(&format!(
                    "missing source PI profile file: {}",
                    source.display()
                ));
            }
            let entry = load_pi_openai_codex(&source);
            let target = ctx.pi_profile_path(target_name);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|e| {
                    crate::data::die(&format!(
                        "failed to create PI profiles directory {}: {}",
                        parent.display(),
                        e
                    ))
                });
            }
            write_pi_openai_codex_profile(&target, &entry);
        }
        ("codex", "pi") => {
            let source = ctx.profile_path(source_name);
            if !source.exists() {
                crate::data::die(&format!(
                    "missing source profile file: {}",
                    source.display()
                ));
            }
            let auth = read_auth(&source);
            let target = ctx.pi_profile_path(target_name);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|e| {
                    crate::data::die(&format!(
                        "failed to create PI profiles directory {}: {}",
                        parent.display(),
                        e
                    ))
                });
            }
            let entry = auth_to_pi_openai_codex(&auth);
            write_pi_openai_codex_profile(&target, &entry);
        }
        ("pi", "codex") => {
            crate::data::die(
                "cannot transfer pi -> codex: PI profiles do not contain the Codex id_token required for a valid Codex auth file",
            );
        }
        _ => crate::data::die("transfer-profile stores must be `codex` or `pi`"),
    }

    println!(
        "Transferred {}:{} -> {}:{}",
        source_store, source_name, target_store, target_name
    );
    println!();
    show_status(ctx, false, false);
}

fn switch_pi_profile_if_present(ctx: &Context, target: &str) {
    let target_pi_path = ctx.pi_profile_path(target);
    if !target_pi_path.exists() {
        println!(
            "No PI auth profile for target {}; left PI auth unchanged",
            target
        );
        return;
    }

    let openai_codex = load_pi_openai_codex(&target_pi_path);
    write_live_pi_openai_codex(ctx, &openai_codex);
    println!("Switched PI auth to profile: {}", target);
}

pub fn restore_last(ctx: &Context) {
    let snapshot = restore_last_snapshot(ctx);
    let profile = snapshot.profile.as_deref().unwrap_or("unknown");
    let observed_at = snapshot.observed_at.as_deref().unwrap_or("unknown");

    println!(
        "Restored live auth from tracker snapshot: profile={} captured_at={}",
        profile, observed_at
    );
    println!();
    show_status(ctx, false, false);
}

pub fn switch_profile(ctx: &Context, target: &str, force: bool, scope: SwitchScope) {
    let target_path = ctx.profile_path(target);
    let target_pi_path = ctx.pi_profile_path(target);
    let should_switch_codex = scope != SwitchScope::PiOnly && target_path.exists();
    let should_switch_pi = scope != SwitchScope::CodexOnly && target_pi_path.exists();

    match scope {
        SwitchScope::Both => {
            if !should_switch_codex && !should_switch_pi {
                crate::data::die(&format!(
                    "missing Codex and PI profile files for target: {}",
                    target
                ));
            }
        }
        SwitchScope::CodexOnly => {
            if !target_path.exists() {
                crate::data::die(&format!("missing profile file: {}", target_path.display()));
            }
        }
        SwitchScope::PiOnly => {
            if !target_pi_path.exists() {
                crate::data::die(&format!(
                    "missing PI profile file: {}",
                    target_pi_path.display()
                ));
            }
        }
    }

    if should_switch_codex {
        if !ctx.live_auth.exists() {
            crate::data::die(&format!(
                "missing live auth file: {}",
                ctx.live_auth.display()
            ));
        }

        let _ = read_auth(&ctx.live_auth);
        let _ = read_auth(&target_path);
    }

    if should_switch_pi {
        let _ = load_pi_openai_codex(&target_pi_path);
    }

    let current_profile = if should_switch_codex {
        detect_current_profile(ctx)
    } else {
        None
    };

    if should_switch_codex && current_profile.is_none() && !force {
        crate::data::die(
            "current auth.json does not match a known profile; use --force to switch anyway",
        );
    }

    if should_switch_codex {
        // Store one reversible recovery snapshot in the tracker before overwriting live auth.
        crate::tracker::snapshot_live_auth(ctx, current_profile.clone());
        println!("Saved recovery snapshot in tracker");

        // Save current auth back into its profile (so refreshed tokens are preserved)
        if let Some(ref name) = current_profile {
            if scope == SwitchScope::Both {
                save_named_profile_state(ctx, name);
            } else {
                save_codex_profile(ctx, name);
            }
        } else {
            println!("Current auth profile is unknown; skipped save-back");
        }

        fs::copy(&target_path, &ctx.live_auth).unwrap_or_else(|e| {
            crate::data::die(&format!("failed to switch to profile {}: {}", target, e))
        });
        println!("Switched live auth to profile: {}", target);
    } else if scope == SwitchScope::Both {
        println!(
            "No Codex auth profile for target {}; left Codex auth unchanged",
            target
        );
    }

    if should_switch_pi {
        switch_pi_profile_if_present(ctx, target);
    } else if scope == SwitchScope::Both {
        println!(
            "No PI auth profile for target {}; left PI auth unchanged",
            target
        );
    }

    println!();
    show_status(ctx, false, false);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        load_pi_openai_codex, restore_last_snapshot, save_codex_profile, save_named_profile_state,
        save_pi_profile_if_present, save_profile, switch_pi_profile_if_present, switch_profile,
        transfer_profile, SwitchScope,
    };
    use crate::data::{AccountTracker, Context, TrackedAuthSnapshot};
    use crate::tracker::save_tracker;

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
        std::fs::create_dir_all(&state_dir).unwrap();
        let ctx = Context {
            live_auth: codex_dir.join("auth.json"),
            pi_auth: base.join(".pi").join("agent").join("auth.json"),
            state_dir: state_dir.clone(),
            tracker_file: state_dir.join("accounts.json"),
        };
        (ctx, base)
    }

    #[test]
    fn restore_last_snapshot_swaps_recovery_state() {
        let (ctx, base) = test_context("restore-last");
        let original_live = r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#;
        let previous_live = r#"{"tokens":{"id_token":"previous","account_id":"acct-prev"}}"#;
        std::fs::write(&ctx.live_auth, original_live).unwrap();

        let mut tracker = AccountTracker::default();
        tracker.last_snapshot = Some(TrackedAuthSnapshot {
            auth_json: previous_live.to_string(),
            observed_at: Some("2026-06-15T00:00:00Z".to_string()),
            profile: Some("work".to_string()),
        });
        save_tracker(&ctx, &tracker);

        let restored = restore_last_snapshot(&ctx);

        assert_eq!(restored.profile.as_deref(), Some("work"));
        assert_eq!(
            std::fs::read_to_string(&ctx.live_auth).unwrap(),
            previous_live
        );

        let swapped_tracker = crate::tracker::load_tracker(&ctx);
        let swapped_snapshot = swapped_tracker.last_snapshot.unwrap();
        assert_eq!(swapped_snapshot.auth_json, original_live);

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn save_named_profile_state_persists_codex_and_pi_profiles() {
        let (ctx, base) = test_context("save-profiles");
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            &ctx.pi_auth,
            r#"{"github-copilot":{"type":"oauth","access":"copilot"},"openai-codex":{"type":"oauth","access":"pi-live"}}"#,
        )
        .unwrap();

        save_named_profile_state(&ctx, "mate");

        assert_eq!(
            std::fs::read_to_string(ctx.profile_path("mate")).unwrap(),
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#
        );
        let saved_pi: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ctx.pi_profile_path("mate")).unwrap())
                .unwrap();
        assert!(saved_pi.get("github-copilot").is_none());
        assert_eq!(saved_pi["openai-codex"]["access"].as_str(), Some("pi-live"));

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn switch_pi_profile_if_present_restores_target_profile() {
        let (ctx, base) = test_context("switch-pi-profile");
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::create_dir_all(ctx.pi_profile_path("me").parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.pi_auth,
            r#"{"github-copilot":{"type":"oauth","access":"copilot"},"openai-codex":{"type":"oauth","access":"old-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            &ctx.pi_profile_path("me"),
            r#"{"openai-codex":{"type":"oauth","access":"pi-target"}}"#,
        )
        .unwrap();

        switch_pi_profile_if_present(&ctx, "me");

        let live_pi: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&ctx.pi_auth).unwrap()).unwrap();
        assert_eq!(
            live_pi["github-copilot"]["access"].as_str(),
            Some("copilot")
        );
        assert_eq!(
            live_pi["openai-codex"]["access"].as_str(),
            Some("pi-target")
        );

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn save_profile_creates_named_snapshots() {
        let (ctx, base) = test_context("save-profile-command");
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            &ctx.pi_auth,
            r#"{"github-copilot":{"type":"oauth","access":"copilot"},"openai-codex":{"type":"oauth","access":"pi-live"}}"#,
        )
        .unwrap();

        save_profile(&ctx, "codex", "manual");
        save_profile(&ctx, "pi", "manual");

        assert!(ctx.profile_path("manual").exists());
        assert!(ctx.pi_profile_path("manual").exists());

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn save_codex_profile_persists_only_codex_snapshot() {
        let (ctx, base) = test_context("save-codex-profile");
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#,
        )
        .unwrap();

        save_codex_profile(&ctx, "mate");

        assert!(ctx.profile_path("mate").exists());
        assert!(!ctx.pi_profile_path("mate").exists());

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn save_pi_profile_if_present_persists_only_pi_snapshot() {
        let (ctx, base) = test_context("save-pi-profile");
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.pi_auth,
            r#"{"github-copilot":{"type":"oauth","access":"copilot"},"openai-codex":{"type":"oauth","access":"pi-live"}}"#,
        )
        .unwrap();

        save_pi_profile_if_present(&ctx, "me");

        assert!(ctx.pi_profile_path("me").exists());
        assert!(!ctx.profile_path("me").exists());
        let saved_pi: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ctx.pi_profile_path("me")).unwrap())
                .unwrap();
        assert!(saved_pi.get("github-copilot").is_none());
        assert_eq!(saved_pi["openai-codex"]["access"].as_str(), Some("pi-live"));

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn save_pi_profile_if_present_skips_alias_for_existing_pi_auth() {
        let (ctx, base) = test_context("save-pi-profile-alias");
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::create_dir_all(ctx.pi_profile_path("me").parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.pi_auth,
            r#"{"openai-codex":{"type":"oauth","access":"pi-live","refresh":"refresh-me","accountId":"acct-me"}}"#,
        )
        .unwrap();
        std::fs::write(
            ctx.pi_profile_path("me"),
            r#"{"openai-codex":{"type":"oauth","access":"pi-saved","refresh":"refresh-me","accountId":"acct-me"}}"#,
        )
        .unwrap();

        save_pi_profile_if_present(&ctx, "mate");

        assert!(ctx.pi_profile_path("me").exists());
        assert!(!ctx.pi_profile_path("mate").exists());

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn switch_profile_codex_only_leaves_pi_auth_unchanged() {
        let (ctx, base) = test_context("switch-codex-only");
        std::fs::create_dir_all(ctx.profile_path("me").parent().unwrap()).unwrap();
        std::fs::create_dir_all(ctx.pi_profile_path("me").parent().unwrap()).unwrap();
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            ctx.profile_path("live"),
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            ctx.profile_path("me"),
            r#"{"tokens":{"id_token":"target","account_id":"acct-target"}}"#,
        )
        .unwrap();
        std::fs::write(&ctx.pi_auth, r#"{"openai-codex":{"access":"pi-live"}}"#).unwrap();
        std::fs::write(
            ctx.pi_profile_path("me"),
            r#"{"openai-codex":{"access":"pi-target"}}"#,
        )
        .unwrap();

        switch_profile(&ctx, "me", false, SwitchScope::CodexOnly);

        assert_eq!(
            std::fs::read_to_string(&ctx.live_auth).unwrap(),
            r#"{"tokens":{"id_token":"target","account_id":"acct-target"}}"#
        );
        let live_pi = load_pi_openai_codex(&ctx.pi_auth);
        assert_eq!(live_pi.access, "pi-live");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn switch_profile_pi_only_leaves_codex_auth_unchanged() {
        let (ctx, base) = test_context("switch-pi-only");
        std::fs::create_dir_all(ctx.pi_profile_path("me").parent().unwrap()).unwrap();
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(&ctx.pi_auth, r#"{"openai-codex":{"access":"pi-live"}}"#).unwrap();
        std::fs::write(
            ctx.pi_profile_path("me"),
            r#"{"openai-codex":{"access":"pi-target"}}"#,
        )
        .unwrap();

        switch_profile(&ctx, "me", false, SwitchScope::PiOnly);

        assert_eq!(
            std::fs::read_to_string(&ctx.live_auth).unwrap(),
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#
        );
        let live_pi = load_pi_openai_codex(&ctx.pi_auth);
        assert_eq!(live_pi.access, "pi-target");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn switch_profile_both_ignores_missing_codex_target_profile() {
        let (ctx, base) = test_context("switch-both-missing-codex-target");
        std::fs::create_dir_all(ctx.pi_profile_path("me").parent().unwrap()).unwrap();
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::write(&ctx.pi_auth, r#"{"openai-codex":{"access":"pi-live"}}"#).unwrap();
        std::fs::write(
            ctx.pi_profile_path("me"),
            r#"{"openai-codex":{"access":"pi-target"}}"#,
        )
        .unwrap();

        switch_profile(&ctx, "me", false, SwitchScope::Both);

        assert!(!ctx.live_auth.exists());
        let live_pi = load_pi_openai_codex(&ctx.pi_auth);
        assert_eq!(live_pi.access, "pi-target");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn switch_profile_both_ignores_missing_pi_target_profile() {
        let (ctx, base) = test_context("switch-both-missing-pi-target");
        std::fs::create_dir_all(ctx.profile_path("me").parent().unwrap()).unwrap();
        std::fs::create_dir_all(ctx.pi_auth.parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            ctx.profile_path("live"),
            r#"{"tokens":{"id_token":"live","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            ctx.profile_path("me"),
            r#"{"tokens":{"id_token":"target","account_id":"acct-target"}}"#,
        )
        .unwrap();
        std::fs::write(&ctx.pi_auth, r#"{"openai-codex":{"access":"pi-live"}}"#).unwrap();

        switch_profile(&ctx, "me", false, SwitchScope::Both);

        assert_eq!(
            std::fs::read_to_string(&ctx.live_auth).unwrap(),
            r#"{"tokens":{"id_token":"target","account_id":"acct-target"}}"#
        );
        let live_pi = load_pi_openai_codex(&ctx.pi_auth);
        assert_eq!(live_pi.access, "pi-live");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn transfer_profile_maps_codex_profile_to_pi_profile() {
        let (ctx, base) = test_context("transfer-codex-to-pi");
        std::fs::create_dir_all(ctx.profile_path("mate").parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"e30.e30.sig","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            ctx.profile_path("mate"),
            r#"{"tokens":{"id_token":"e30.e30.sig","access_token":"e30.eyJleHAiOjE3ODIwODQ2NzR9.sig","refresh_token":"refresh-token","account_id":"acct-mate"}}"#,
        )
        .unwrap();

        transfer_profile(&ctx, "codex", "mate", "pi", "mate-pi");

        let saved_pi = load_pi_openai_codex(&ctx.pi_profile_path("mate-pi"));
        assert_eq!(saved_pi.account_id.as_deref(), Some("acct-mate"));
        assert_eq!(saved_pi.refresh.as_deref(), Some("refresh-token"));
        assert_eq!(saved_pi.expires, Some(1_782_084_674_000));

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn transfer_profile_copies_pi_profile_without_other_entries() {
        let (ctx, base) = test_context("transfer-pi-to-pi");
        std::fs::create_dir_all(ctx.pi_profile_path("me").parent().unwrap()).unwrap();
        std::fs::write(
            &ctx.live_auth,
            r#"{"tokens":{"id_token":"e30.e30.sig","account_id":"acct-live"}}"#,
        )
        .unwrap();
        std::fs::write(
            ctx.pi_profile_path("me"),
            r#"{"openai-codex":{"type":"oauth","access":"pi-access","refresh":"pi-refresh","accountId":"acct-pi"}}"#,
        )
        .unwrap();

        transfer_profile(&ctx, "pi", "me", "pi", "clone");

        let saved_pi = load_pi_openai_codex(&ctx.pi_profile_path("clone"));
        assert_eq!(saved_pi.access, "pi-access");
        assert_eq!(saved_pi.refresh.as_deref(), Some("pi-refresh"));

        std::fs::remove_dir_all(base).unwrap();
    }
}

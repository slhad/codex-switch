use crate::data::Context;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AutoSwitchPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub codex: bool,
    #[serde(default = "default_true")]
    pub pi: bool,
}

impl Default for AutoSwitchPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            priority: 0,
            codex: true,
            pi: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TransferOnSwitchPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub pi_profile: String,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProfileOption {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto: Option<AutoSwitchPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<TransferOnSwitchPolicy>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProfileOptions {
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileOption>,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyAutoProfiles {
    #[serde(default)]
    profiles: BTreeMap<String, AutoSwitchPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyProfileTransfers {
    #[serde(default)]
    codex_to_pi: BTreeMap<String, String>,
}

fn default_true() -> bool {
    true
}

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {}", path.display(), error))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("invalid profile options {}: {}", path.display(), error))
}

fn load_with_migration(ctx: &Context, persist_migration: bool) -> Result<ProfileOptions, String> {
    let path = ctx.profile_options_path();
    if path.exists() {
        return read_json(&path);
    }

    let legacy_auto_path = ctx.state_dir.join("auto-profiles.json");
    let legacy_transfers_path = ctx.state_dir.join("profile-transfers.json");
    let mut options = ProfileOptions::default();
    let mut migrated = false;

    if legacy_auto_path.exists() {
        let legacy: LegacyAutoProfiles = read_json(&legacy_auto_path)?;
        for (name, auto) in legacy.profiles {
            options.profiles.entry(name).or_default().auto = Some(auto);
        }
        migrated = true;
    }

    if legacy_transfers_path.exists() {
        let legacy: LegacyProfileTransfers = read_json(&legacy_transfers_path)?;
        for (name, pi_profile) in legacy.codex_to_pi {
            options.profiles.entry(name).or_default().transfer = Some(TransferOnSwitchPolicy {
                enabled: true,
                pi_profile,
            });
        }
        migrated = true;
    }

    if migrated && persist_migration {
        save(ctx, &options)?;
    }
    Ok(options)
}

pub fn load(ctx: &Context) -> Result<ProfileOptions, String> {
    load_with_migration(ctx, true)
}

pub fn load_read_only(ctx: &Context) -> Result<ProfileOptions, String> {
    load_with_migration(ctx, false)
}

pub fn save(ctx: &Context, options: &ProfileOptions) -> Result<(), String> {
    std::fs::create_dir_all(&ctx.state_dir).map_err(|error| {
        format!(
            "failed to create state directory {}: {}",
            ctx.state_dir.display(),
            error
        )
    })?;
    let path = ctx.profile_options_path();
    let content = serde_json::to_string_pretty(options)
        .map_err(|error| format!("failed to serialize profile options: {}", error))?;
    std::fs::write(&path, format!("{}\n", content)).map_err(|error| {
        format!(
            "failed to save profile options {}: {}",
            path.display(),
            error
        )
    })
}

pub fn update_auto(
    ctx: &Context,
    name: &str,
    enabled: Option<bool>,
    priority: Option<i32>,
    codex: Option<bool>,
    pi: Option<bool>,
) -> Result<AutoSwitchPolicy, String> {
    crate::switch::validate_profile_name(name)?;
    let mut options = load(ctx)?;
    let auto = options
        .profiles
        .entry(name.to_string())
        .or_default()
        .auto
        .get_or_insert_with(AutoSwitchPolicy::default);
    if let Some(enabled) = enabled {
        auto.enabled = enabled;
    }
    if let Some(priority) = priority {
        auto.priority = priority;
    }
    if let Some(codex) = codex {
        auto.codex = codex;
    }
    if let Some(pi) = pi {
        auto.pi = pi;
    }
    let auto = auto.clone();
    save(ctx, &options)?;
    Ok(auto)
}

pub fn remove_auto(ctx: &Context, name: &str) -> Result<bool, String> {
    crate::switch::validate_profile_name(name)?;
    let mut options = load(ctx)?;
    let removed = options
        .profiles
        .get_mut(name)
        .and_then(|option| option.auto.take())
        .is_some();
    if options
        .profiles
        .get(name)
        .is_some_and(|option| option.auto.is_none() && option.transfer.is_none())
    {
        options.profiles.remove(name);
    }
    if removed {
        save(ctx, &options)?;
    }
    Ok(removed)
}

pub fn configure_transfer(ctx: &Context, name: &str, pi_profile: &str) -> Result<(), String> {
    crate::switch::validate_profile_name(name)?;
    crate::switch::validate_profile_name(pi_profile)?;
    let mut options = load(ctx)?;
    options
        .profiles
        .entry(name.to_string())
        .or_default()
        .transfer = Some(TransferOnSwitchPolicy {
        enabled: true,
        pi_profile: pi_profile.to_string(),
    });
    save(ctx, &options)
}

pub fn set_transfer_enabled(ctx: &Context, name: &str, enabled: bool) -> Result<String, String> {
    crate::switch::validate_profile_name(name)?;
    let mut options = load(ctx)?;
    let transfer = options
        .profiles
        .get_mut(name)
        .and_then(|option| option.transfer.as_mut())
        .ok_or_else(|| {
            format!(
                "profile `{}` has no transfer target; configure one with `profile transfer on-switch set {} PI_PROFILE`",
                name, name
            )
        })?;
    transfer.enabled = enabled;
    let target = transfer.pi_profile.clone();
    save(ctx, &options)?;
    Ok(target)
}

pub fn transfer_target(ctx: &Context, name: &str) -> Result<Option<String>, String> {
    Ok(load(ctx)?
        .profiles
        .get(name)
        .and_then(|option| option.transfer.as_ref())
        .filter(|transfer| transfer.enabled)
        .map(|transfer| transfer.pi_profile.clone()))
}

#[cfg(test)]
mod tests {
    use super::{
        configure_transfer, load, load_read_only, set_transfer_enabled, transfer_target,
        update_auto, AutoSwitchPolicy, TransferOnSwitchPolicy,
    };
    use crate::data::Context;

    fn context(name: &str) -> (Context, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "codex-switch-profile-options-{}-{}",
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

    #[test]
    fn migrates_legacy_auto_and_transfer_files_into_one_config() {
        let (ctx, base) = context("migration");
        std::fs::create_dir_all(&ctx.state_dir).unwrap();
        std::fs::write(
            ctx.state_dir.join("auto-profiles.json"),
            r#"{"profiles":{"work":{"enabled":false,"priority":50,"codex":true,"pi":false}}}"#,
        )
        .unwrap();
        std::fs::write(
            ctx.state_dir.join("profile-transfers.json"),
            r#"{"codex_to_pi":{"work":"work-pi","mate":"mate"}}"#,
        )
        .unwrap();

        let options = load(&ctx).unwrap();
        assert_eq!(
            options.profiles["work"].auto,
            Some(AutoSwitchPolicy {
                enabled: false,
                priority: 50,
                codex: true,
                pi: false,
            })
        );
        assert_eq!(
            options.profiles["work"].transfer,
            Some(TransferOnSwitchPolicy {
                enabled: true,
                pi_profile: "work-pi".to_string(),
            })
        );
        assert!(options.profiles["mate"].auto.is_none());
        assert!(ctx.profile_options_path().exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn read_only_load_does_not_persist_legacy_migration() {
        let (ctx, base) = context("read-only-migration");
        std::fs::create_dir_all(&ctx.state_dir).unwrap();
        std::fs::write(
            ctx.state_dir.join("auto-profiles.json"),
            r#"{"profiles":{"work":{"priority":50}}}"#,
        )
        .unwrap();
        assert_eq!(
            load_read_only(&ctx).unwrap().profiles["work"]
                .auto
                .as_ref()
                .unwrap()
                .priority,
            50
        );
        assert!(!ctx.profile_options_path().exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn configures_and_toggles_transfer_without_copying_auth() {
        let (ctx, base) = context("transfer");
        configure_transfer(&ctx, "work", "work-pi").unwrap();
        assert_eq!(
            transfer_target(&ctx, "work").unwrap().as_deref(),
            Some("work-pi")
        );
        assert_eq!(
            set_transfer_enabled(&ctx, "work", false).unwrap(),
            "work-pi"
        );
        assert!(transfer_target(&ctx, "work").unwrap().is_none());
        assert_eq!(set_transfer_enabled(&ctx, "work", true).unwrap(), "work-pi");
        assert!(set_transfer_enabled(&ctx, "missing", true).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn auto_policy_is_optional_for_transfer_only_profiles() {
        let (ctx, base) = context("optional-auto");
        configure_transfer(&ctx, "work", "work").unwrap();
        assert!(load(&ctx).unwrap().profiles["work"].auto.is_none());
        update_auto(&ctx, "work", None, Some(10), None, None).unwrap();
        assert_eq!(
            load(&ctx).unwrap().profiles["work"]
                .auto
                .as_ref()
                .unwrap()
                .priority,
            10
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}

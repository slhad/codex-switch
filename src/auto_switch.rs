use crate::data::{read_pi_auth, Context, UsageResponse};
use crate::jwt::extract_email_from_token;
use crate::profile::{list_pi_profiles, profile_name};
use crate::profile_options::ProfileOptions;
use crate::rate_limit::{
    fetch_pi_rate_limit, fetch_pi_rate_limit_for_auth_read_only, fetch_pi_rate_limit_for_path,
    fetch_pi_rate_limit_read_only, fetch_rate_limit, fetch_rate_limit_for_auth_path,
    fetch_rate_limit_for_auth_path_read_only, fetch_rate_limit_read_only,
};
use crate::switch::{switch_profile_with_status, validate_profile_name, SwitchScope};
use std::collections::BTreeMap;

pub const THRESHOLD_ENV: &str = "CODEX_SWITCH_THRESHOLD_PERCENT";
pub const THRESHOLD_STEP_ENV: &str = "CODEX_SWITCH_THRESHOLD_STEP_PERCENT";
const DEFAULT_THRESHOLD: f64 = 90.0;
const DEFAULT_THRESHOLD_STEP: f64 = 5.0;

#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    name: String,
    priority: i32,
    used_percent: f64,
}

fn percentage_from_env(name: &str, default: f64, minimum: f64) -> Result<f64, String> {
    let Some(raw) = std::env::var_os(name) else {
        return Ok(default);
    };
    let value = raw
        .to_string_lossy()
        .parse::<f64>()
        .map_err(|_| format!("{} must be a number from {} to 100", name, minimum))?;
    if !value.is_finite() || !(minimum..=100.0).contains(&value) {
        return Err(format!("{} must be a number from {} to 100", name, minimum));
    }
    Ok(value)
}

pub fn threshold_from_env() -> Result<f64, String> {
    percentage_from_env(THRESHOLD_ENV, DEFAULT_THRESHOLD, 0.0)
}

pub fn threshold_step_from_env() -> Result<f64, String> {
    let value = percentage_from_env(THRESHOLD_STEP_ENV, DEFAULT_THRESHOLD_STEP, 0.0)?;
    if value == 0.0 {
        return Err(format!("{} must be greater than 0", THRESHOLD_STEP_ENV));
    }
    Ok(value)
}

fn usage_percent(usage: &UsageResponse) -> Option<f64> {
    let mut values = Vec::new();
    if let Some(value) = usage.monthly_limit().and_then(|window| window.used_percent) {
        values.push(value);
    }
    if let Some(value) = usage
        .five_hour_window()
        .and_then(|window| window.used_percent)
    {
        values.push(value);
    }
    if let Some(value) = usage.weekly_window().and_then(|window| window.used_percent) {
        values.push(value);
    }
    values.into_iter().reduce(f64::max)
}

fn current_profile_is_enabled(
    policies: &BTreeMap<String, crate::profile_options::ProfileOption>,
    current: &str,
    codex: bool,
) -> bool {
    policies
        .get(current)
        .and_then(|option| option.auto.as_ref())
        .is_some_and(|policy| policy.enabled && if codex { policy.codex } else { policy.pi })
}

fn choose_candidate(
    candidates: Vec<Candidate>,
    threshold: f64,
    threshold_step: f64,
    current_used_percent: f64,
) -> Option<(Candidate, f64)> {
    let mut eligible: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| candidate.used_percent < current_used_percent)
        .filter_map(|candidate| {
            let ceiling = if candidate.used_percent < threshold {
                threshold
            } else {
                let increments =
                    ((candidate.used_percent - threshold) / threshold_step).floor() + 1.0;
                (threshold + increments * threshold_step).min(100.0)
            };
            (candidate.used_percent < ceiling).then_some((candidate, ceiling))
        })
        .collect();

    eligible.sort_by(|(left, left_ceiling), (right, right_ceiling)| {
        left_ceiling
            .total_cmp(right_ceiling)
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| left.used_percent.total_cmp(&right.used_percent))
            .then_with(|| left.name.cmp(&right.name))
    });
    eligible.into_iter().next()
}

pub fn set_profile_policy(
    ctx: &Context,
    name: &str,
    enabled: Option<bool>,
    priority: Option<i32>,
    codex: Option<bool>,
    pi: Option<bool>,
) -> Result<(), String> {
    validate_profile_name(name)?;
    let policy = crate::profile_options::update_auto(ctx, name, enabled, priority, codex, pi)?;
    println!(
        "Automatic profile {}: enabled={} priority={} codex={} pi={}",
        name, policy.enabled, policy.priority, policy.codex, policy.pi
    );
    Ok(())
}

pub fn remove_profile_policy(ctx: &Context, name: &str) -> Result<(), String> {
    if crate::profile_options::remove_auto(ctx, name)? {
        println!("Removed automatic profile policy: {}", name);
    } else {
        println!("No automatic profile policy found: {}", name);
    }
    Ok(())
}

fn profile_option_lines(options: &ProfileOptions) -> Vec<String> {
    if options.profiles.is_empty() {
        return vec!["  none".to_string()];
    }
    options
        .profiles
        .iter()
        .map(|(name, option)| {
            let mut parts = Vec::new();
            if let Some(policy) = option.auto.as_ref() {
                parts.push(format!(
                    "auto enabled={} priority={} codex={} pi={}",
                    policy.enabled, policy.priority, policy.codex, policy.pi
                ));
            }
            if let Some(transfer) = option.transfer.as_ref() {
                parts.push(format!(
                    "transfer enabled={} pi_profile={}",
                    transfer.enabled, transfer.pi_profile
                ));
            }
            format!("  {}: {}", name, parts.join("; "))
        })
        .collect()
}

pub fn print_profile_options(ctx: &Context) -> Result<(), String> {
    let config = crate::profile_options::load(ctx)?;
    println!();
    println!("Profile options:");
    for line in profile_option_lines(&config) {
        println!("{}", line);
    }
    Ok(())
}

pub fn show_config(ctx: &Context) -> Result<(), String> {
    let threshold = threshold_from_env()?;
    let threshold_step = threshold_step_from_env()?;
    let config = crate::profile_options::load(ctx)?;
    println!("Automatic switching:");
    println!("  threshold: {}% ({})", threshold, THRESHOLD_ENV);
    println!(
        "  threshold step: {}% ({})",
        threshold_step, THRESHOLD_STEP_ENV
    );
    println!("  options file: {}", ctx.profile_options_path().display());
    println!("  profiles:");
    for line in profile_option_lines(&config) {
        println!("  {}", line);
    }
    Ok(())
}

fn codex_candidates(
    ctx: &Context,
    policies: &BTreeMap<String, crate::profile_options::ProfileOption>,
    current: &str,
    dry_run: bool,
) -> Vec<Candidate> {
    policies
        .iter()
        .filter_map(|(name, option)| option.auto.as_ref().map(|policy| (name, policy)))
        .filter(|(name, policy)| policy.enabled && policy.codex && name.as_str() != current)
        .filter_map(|(name, policy)| {
            let path = ctx.profile_path(name);
            if !path.exists() {
                eprintln!("Skipping Codex profile {}: profile file is missing", name);
                return None;
            }
            let usage = if dry_run {
                fetch_rate_limit_for_auth_path_read_only(&path)
            } else {
                fetch_rate_limit_for_auth_path(&path).map(|(usage, _)| usage)
            };
            match usage {
                Ok(usage) => usage_percent(&usage).map(|used_percent| Candidate {
                    name: name.clone(),
                    priority: policy.priority,
                    used_percent,
                }),
                Err(error) => {
                    eprintln!("Skipping Codex profile {}: {}", name, error);
                    None
                }
            }
        })
        .collect()
}

fn detect_current_pi_profile(ctx: &Context) -> Option<String> {
    let live = read_pi_auth(&ctx.pi_auth)?.openai_codex?;
    let live_email = extract_email_from_token(&live.access);
    for path in list_pi_profiles(ctx) {
        let Some(saved) = read_pi_auth(&path).and_then(|auth| auth.openai_codex) else {
            continue;
        };
        if live.account_id.is_some() && live.account_id == saved.account_id {
            return Some(profile_name(&path));
        }
        if live_email.is_some() && live_email == extract_email_from_token(&saved.access) {
            return Some(profile_name(&path));
        }
    }
    None
}

fn pi_candidates(
    ctx: &Context,
    policies: &BTreeMap<String, crate::profile_options::ProfileOption>,
    current: &str,
    dry_run: bool,
) -> Vec<Candidate> {
    policies
        .iter()
        .filter_map(|(name, option)| option.auto.as_ref().map(|policy| (name, policy)))
        .filter(|(name, policy)| policy.enabled && policy.pi && name.as_str() != current)
        .filter_map(|(name, policy)| {
            let path = ctx.pi_profile_path(name);
            let Some(auth) = read_pi_auth(&path).and_then(|auth| auth.openai_codex) else {
                eprintln!(
                    "Skipping PI profile {}: profile file is missing or invalid",
                    name
                );
                return None;
            };
            let usage = if dry_run {
                fetch_pi_rate_limit_for_auth_read_only(&auth)
            } else {
                fetch_pi_rate_limit_for_path(&path, auth).map(|(usage, _)| usage)
            };
            match usage {
                Ok(usage) => usage_percent(&usage).map(|used_percent| Candidate {
                    name: name.clone(),
                    priority: policy.priority,
                    used_percent,
                }),
                Err(error) => {
                    eprintln!("Skipping PI profile {}: {}", name, error);
                    None
                }
            }
        })
        .collect()
}

pub fn run(ctx: &Context, dry_run: bool) -> Result<(), String> {
    let threshold = threshold_from_env()?;
    let threshold_step = threshold_step_from_env()?;
    let config = if dry_run {
        crate::profile_options::load_read_only(ctx)?
    } else {
        crate::profile_options::load(ctx)?
    };
    if !config.profiles.values().any(|option| option.auto.is_some()) {
        println!("Automatic switch skipped: no profile policies configured");
        return Ok(());
    }

    let mut codex_target = None;
    if ctx.live_auth.exists() {
        if let Some(current) = crate::profile::detect_current_profile(ctx) {
            if !current_profile_is_enabled(&config.profiles, &current, true) {
                println!(
                    "Codex automatic switch skipped: current profile {} is not enabled for automatic switching",
                    current
                );
            } else {
                let current_usage = if dry_run {
                    fetch_rate_limit_read_only(ctx)
                } else {
                    fetch_rate_limit(ctx)
                };
                match current_usage {
                    Ok(usage) => match usage_percent(&usage) {
                        Some(used) if used >= threshold => {
                            codex_target = choose_candidate(
                                codex_candidates(ctx, &config.profiles, &current, dry_run),
                                threshold,
                                threshold_step,
                                used,
                            );
                            if codex_target.is_none() {
                                println!(
                                    "Codex usage is {}% (threshold {}%); no eligible profile has lower usage",
                                    used, threshold
                                );
                            }
                        }
                        Some(used) => println!(
                            "Codex usage is {}%; below automatic-switch threshold {}%",
                            used, threshold
                        ),
                        None => {
                            println!("Codex automatic switch skipped: usage percentage unavailable")
                        }
                    },
                    Err(error) => eprintln!("Codex automatic switch skipped: {}", error),
                }
            }
        } else {
            eprintln!(
                "Codex automatic switch skipped: current {} is not a known profile",
                ctx.live_auth.display()
            );
        }
    }

    let mut pi_target = None;
    if let Some(live_pi) = read_pi_auth(&ctx.pi_auth).and_then(|auth| auth.openai_codex) {
        if let Some(current) = detect_current_pi_profile(ctx) {
            if !current_profile_is_enabled(&config.profiles, &current, false) {
                println!(
                    "PI automatic switch skipped: current profile {} is not enabled for automatic switching",
                    current
                );
            } else {
                let current_usage = if dry_run {
                    fetch_pi_rate_limit_read_only(&live_pi)
                } else {
                    fetch_pi_rate_limit(ctx, live_pi).map(|(usage, _)| usage)
                };
                match current_usage {
                    Ok(usage) => match usage_percent(&usage) {
                        Some(used) if used >= threshold => {
                            pi_target = choose_candidate(
                                pi_candidates(ctx, &config.profiles, &current, dry_run),
                                threshold,
                                threshold_step,
                                used,
                            );
                            if pi_target.is_none() {
                                println!(
                                    "PI usage is {}% (threshold {}%); no eligible profile has lower usage",
                                    used, threshold
                                );
                            }
                        }
                        Some(used) => println!(
                            "PI usage is {}%; below automatic-switch threshold {}%",
                            used, threshold
                        ),
                        None => {
                            println!("PI automatic switch skipped: usage percentage unavailable")
                        }
                    },
                    Err(error) => eprintln!("PI automatic switch skipped: {}", error),
                }
            }
        } else {
            eprintln!(
                "PI automatic switch skipped: current {} is not a known profile",
                ctx.pi_auth.display()
            );
        }
    }

    if let Some((target, effective_threshold)) = codex_target {
        if dry_run {
            println!(
                "Dry run: would switch Codex to {} (priority {}, usage {}%, selection threshold {}%)",
                target.name, target.priority, target.used_percent, effective_threshold
            );
        } else {
            println!(
                "Switching Codex to {} (priority {}, usage {}%, selection threshold {}%)",
                target.name, target.priority, target.used_percent, effective_threshold
            );
            switch_profile_with_status(ctx, &target.name, false, SwitchScope::CodexOnly, false);
        }
    }
    if let Some((target, effective_threshold)) = pi_target {
        if dry_run {
            println!(
                "Dry run: would switch PI to {} (priority {}, usage {}%, selection threshold {}%)",
                target.name, target.priority, target.used_percent, effective_threshold
            );
        } else {
            println!(
                "Switching PI to {} (priority {}, usage {}%, selection threshold {}%)",
                target.name, target.priority, target.used_percent, effective_threshold
            );
            switch_profile_with_status(ctx, &target.name, false, SwitchScope::PiOnly, false);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        choose_candidate, codex_candidates, current_profile_is_enabled, detect_current_pi_profile,
        percentage_from_env, pi_candidates, print_profile_options, profile_option_lines,
        remove_profile_policy, run, set_profile_policy, show_config, usage_percent, Candidate,
    };
    use crate::data::{Context, UsageResponse};
    use crate::profile_options::{
        AutoSwitchPolicy, ProfileOption, ProfileOptions, TransferOnSwitchPolicy,
    };

    fn context(name: &str) -> (Context, std::path::PathBuf) {
        let base =
            std::env::temp_dir().join(format!("codex-switch-auto-{}-{}", name, std::process::id()));
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
    fn validates_percentage_environment_values() {
        let name = "CODEX_SWITCH_TEST_PERCENTAGE";
        std::env::remove_var(name);
        assert_eq!(percentage_from_env(name, 90.0, 0.0).unwrap(), 90.0);
        for (value, expected) in [
            ("42.5", Some(42.5)),
            ("nope", None),
            ("NaN", None),
            ("101", None),
            ("-1", None),
        ] {
            std::env::set_var(name, value);
            assert_eq!(percentage_from_env(name, 90.0, 0.0).ok(), expected);
        }
        std::env::remove_var(name);
    }

    #[test]
    fn command_helpers_handle_empty_and_missing_profiles() {
        let (ctx, base) = context("command-helpers");
        print_profile_options(&ctx).unwrap();
        show_config(&ctx).unwrap();
        run(&ctx, true).unwrap();
        assert!(detect_current_pi_profile(&ctx).is_none());

        let mut options = ProfileOptions::default();
        options.profiles.insert(
            "missing".to_string(),
            ProfileOption {
                auto: Some(AutoSwitchPolicy::default()),
                transfer: None,
            },
        );
        assert!(codex_candidates(&ctx, &options.profiles, "current", true).is_empty());
        assert!(pi_candidates(&ctx, &options.profiles, "current", true).is_empty());

        remove_profile_policy(&ctx, "missing").unwrap();
        set_profile_policy(&ctx, "work", None, None, None, None).unwrap();
        remove_profile_policy(&ctx, "work").unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn formats_profile_options_for_status_output() {
        assert_eq!(
            profile_option_lines(&ProfileOptions::default()),
            vec!["  none"]
        );
        let mut config = ProfileOptions::default();
        config.profiles.insert(
            "work".to_string(),
            ProfileOption {
                auto: Some(AutoSwitchPolicy {
                    enabled: true,
                    priority: 100,
                    codex: true,
                    pi: false,
                }),
                transfer: Some(TransferOnSwitchPolicy {
                    enabled: false,
                    pi_profile: "work-pi".to_string(),
                }),
            },
        );
        assert_eq!(
            profile_option_lines(&config),
            vec!["  work: auto enabled=true priority=100 codex=true pi=false; transfer enabled=false pi_profile=work-pi"]
        );
    }

    #[test]
    fn combined_policy_command_updates_the_named_profile() {
        let (ctx, base) = context("combined-policy");
        set_profile_policy(&ctx, "work", Some(false), Some(42), Some(false), Some(true)).unwrap();

        let config = crate::profile_options::load(&ctx).unwrap();
        assert_eq!(
            config.profiles["work"].auto,
            Some(AutoSwitchPolicy {
                enabled: false,
                priority: 42,
                codex: false,
                pi: true,
            })
        );
        assert!(set_profile_policy(&ctx, "bad/name", None, Some(1), None, None).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn auto_profile_policy_defaults_to_enabled_for_both_targets() {
        let policy: AutoSwitchPolicy = serde_json::from_str("{}").unwrap();
        assert_eq!(policy, AutoSwitchPolicy::default());
    }

    #[test]
    fn current_profile_must_be_enabled_for_the_target_store() {
        let mut config = ProfileOptions::default();
        assert!(!current_profile_is_enabled(
            &config.profiles,
            "missing",
            true
        ));

        config.profiles.insert(
            "work".to_string(),
            ProfileOption {
                auto: Some(AutoSwitchPolicy {
                    enabled: false,
                    priority: 0,
                    codex: true,
                    pi: true,
                }),
                transfer: None,
            },
        );
        assert!(!current_profile_is_enabled(&config.profiles, "work", true));

        config.profiles.get_mut("work").unwrap().auto = Some(AutoSwitchPolicy {
            enabled: true,
            priority: 0,
            codex: true,
            pi: false,
        });
        assert!(current_profile_is_enabled(&config.profiles, "work", true));
        assert!(!current_profile_is_enabled(&config.profiles, "work", false));
    }

    #[test]
    fn usage_uses_highest_available_consumption_window() {
        let usage: UsageResponse = serde_json::from_str(
            r#"{"rate_limit":{"primary_window":{"used_percent":20,"limit_window_seconds":18000},"secondary_window":{"used_percent":75,"limit_window_seconds":604800}}}"#,
        )
        .unwrap();
        assert_eq!(usage_percent(&usage), Some(75.0));
    }

    #[test]
    fn candidate_selection_prefers_priority_then_usage_then_name() {
        let candidates = vec![
            Candidate {
                name: "low".to_string(),
                priority: 1,
                used_percent: 5.0,
            },
            Candidate {
                name: "zeta".to_string(),
                priority: 10,
                used_percent: 20.0,
            },
            Candidate {
                name: "alpha".to_string(),
                priority: 10,
                used_percent: 20.0,
            },
            Candidate {
                name: "full".to_string(),
                priority: 100,
                used_percent: 90.0,
            },
        ];
        let (candidate, effective_threshold) =
            choose_candidate(candidates, 90.0, 5.0, 95.0).unwrap();
        assert_eq!(candidate.name, "alpha");
        assert_eq!(effective_threshold, 90.0);
    }

    #[test]
    fn candidate_selection_steps_until_a_lower_usage_profile_is_available() {
        let (candidate, effective_threshold) = choose_candidate(
            vec![
                Candidate {
                    name: "higher".to_string(),
                    priority: 1,
                    used_percent: 94.0,
                },
                Candidate {
                    name: "lower".to_string(),
                    priority: 1,
                    used_percent: 91.0,
                },
            ],
            85.0,
            5.0,
            98.0,
        )
        .unwrap();
        assert_eq!(candidate.name, "lower");
        assert_eq!(effective_threshold, 95.0);
    }

    #[test]
    fn candidate_selection_returns_none_without_a_usage_improvement() {
        assert!(choose_candidate(
            vec![Candidate {
                name: "same".to_string(),
                priority: 1,
                used_percent: 90.0,
            }],
            85.0,
            5.0,
            90.0,
        )
        .is_none());
    }
}

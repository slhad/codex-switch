use crate::auto_switch::{THRESHOLD_ENV, THRESHOLD_STEP_ENV};
use std::path::{Path, PathBuf};
use std::process::Command;

const SERVICE_NAME: &str = "codex-switch.service";
const TIMER_NAME: &str = "codex-switch.timer";
const TIMER_BOOT_DELAY_ENV: &str = "CODEX_SWITCH_TIMER_BOOT_DELAY";
const TIMER_INTERVAL_ENV: &str = "CODEX_SWITCH_TIMER_INTERVAL";
const DEFAULT_TIMER_BOOT_DELAY: &str = "2min";
const DEFAULT_TIMER_INTERVAL: &str = "5min";

fn user_systemd_dir(home: &Path) -> PathBuf {
    home.join(".config").join("systemd").join("user")
}

fn unit_setting<'a>(content: &'a str, name: &str) -> Option<&'a str> {
    content
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{}=", name)))
}

fn service_environment<'a>(content: &'a str, name: &str) -> Option<&'a str> {
    content.lines().find_map(|line| {
        line.strip_prefix("Environment=")?
            .strip_prefix(&format!("{}=", name))
    })
}

fn installation_lines(home: &Path) -> Result<Option<Vec<String>>, String> {
    let unit_dir = user_systemd_dir(home);
    let service_path = unit_dir.join(SERVICE_NAME);
    let timer_path = unit_dir.join(TIMER_NAME);
    if !service_path.exists() && !timer_path.exists() {
        return Ok(None);
    }

    let mut lines = Vec::new();
    if service_path.exists() {
        let content = std::fs::read_to_string(&service_path)
            .map_err(|error| format!("failed to read {}: {}", service_path.display(), error))?;
        lines.push(format!("  service: installed ({})", service_path.display()));
        if let Some(value) = service_environment(&content, THRESHOLD_ENV) {
            lines.push(format!("  threshold: {}%", value));
        }
        if let Some(value) = service_environment(&content, THRESHOLD_STEP_ENV) {
            lines.push(format!("  threshold step: {}%", value));
        }
    } else {
        lines.push(format!("  service: missing ({})", service_path.display()));
    }

    if timer_path.exists() {
        let content = std::fs::read_to_string(&timer_path)
            .map_err(|error| format!("failed to read {}: {}", timer_path.display(), error))?;
        lines.push(format!("  timer: installed ({})", timer_path.display()));
        if let Some(value) = unit_setting(&content, "OnBootSec") {
            lines.push(format!("  boot delay: {}", value));
        }
        if let Some(value) = unit_setting(&content, "OnUnitActiveSec") {
            lines.push(format!("  interval: {}", value));
        }
        if let Some(value) = unit_setting(&content, "Persistent") {
            lines.push(format!("  persistent: {}", value));
        }
    } else {
        lines.push(format!("  timer: missing ({})", timer_path.display()));
    }

    Ok(Some(lines))
}

pub fn print_installation_status() -> Result<(), String> {
    let home = PathBuf::from(std::env::var("HOME").map_err(|_| "HOME not set".to_string())?);
    let Some(lines) = installation_lines(&home)? else {
        return Ok(());
    };
    println!();
    println!("Systemd auto-switch:");
    for line in lines {
        println!("{}", line);
    }
    Ok(())
}

fn quote_systemd(value: &Path) -> String {
    let value = value.to_string_lossy().replace('%', "%%");
    format!("\"{}\"", value.replace('\\', "\\\\").replace('\"', "\\\""))
}

fn service_content(executable: &Path, threshold: f64, threshold_step: f64) -> String {
    format!(
        "[Unit]\nDescription=Automatically switch Codex and PI OAuth profiles by usage\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=oneshot\nEnvironment={}={}\nEnvironment={}={}\nExecStart={} auto run\n",
        THRESHOLD_ENV,
        threshold,
        THRESHOLD_STEP_ENV,
        threshold_step,
        quote_systemd(executable)
    )
}

fn validate_timer_value(name: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '.' || character == ' '
        })
    {
        return Err(format!(
            "{} must be a systemd duration such as `30s`, `5min`, or `1h 30min`",
            name
        ));
    }
    Ok(value.to_string())
}

fn timer_value_from_env(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) => validate_timer_value(name, &value),
        Err(std::env::VarError::NotPresent) => Ok(default.to_string()),
        Err(error) => Err(format!("invalid {}: {}", name, error)),
    }
}

fn timer_content(boot_delay: &str, interval: &str) -> String {
    format!(
        "[Unit]\nDescription=Check Codex and PI OAuth profile usage periodically\n\n[Timer]\nOnBootSec={}\nOnUnitActiveSec={}\nPersistent=true\nUnit=codex-switch.service\n\n[Install]\nWantedBy=timers.target\n",
        boot_delay, interval
    )
}

fn run(command: &mut Command, description: &str, allow_failure: bool) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("failed to {}: {}", description, error))?;
    if !status.success() && !allow_failure {
        return Err(format!(
            "failed to {} (exit status {})",
            description, status
        ));
    }
    Ok(())
}

fn systemctl(args: &[&str], allow_failure: bool) -> Result<(), String> {
    run(
        Command::new("systemctl").arg("--user").args(args),
        &format!("run systemctl --user {}", args.join(" ")),
        allow_failure,
    )
}

pub fn install() -> Result<(), String> {
    let home = PathBuf::from(std::env::var("HOME").map_err(|_| "HOME not set".to_string())?);
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {}", error))?;
    let threshold = crate::auto_switch::threshold_from_env()?;
    let threshold_step = crate::auto_switch::threshold_step_from_env()?;
    let timer_boot_delay = timer_value_from_env(TIMER_BOOT_DELAY_ENV, DEFAULT_TIMER_BOOT_DELAY)?;
    let timer_interval = timer_value_from_env(TIMER_INTERVAL_ENV, DEFAULT_TIMER_INTERVAL)?;
    install_files(
        &home,
        &executable,
        threshold,
        threshold_step,
        &timer_boot_delay,
        &timer_interval,
    )?;
    systemctl(&["daemon-reload"], false)?;
    systemctl(&["enable", "--now", TIMER_NAME], false)?;
    println!("Installed and started systemd user timer: {}", TIMER_NAME);
    println!("Service threshold: {}={}", THRESHOLD_ENV, threshold);
    println!(
        "Service threshold step: {}={}",
        THRESHOLD_STEP_ENV, threshold_step
    );
    println!(
        "Timer boot delay: {}={}",
        TIMER_BOOT_DELAY_ENV, timer_boot_delay
    );
    println!("Timer interval: {}={}", TIMER_INTERVAL_ENV, timer_interval);
    Ok(())
}

fn install_files(
    home: &Path,
    executable: &Path,
    threshold: f64,
    threshold_step: f64,
    timer_boot_delay: &str,
    timer_interval: &str,
) -> Result<(), String> {
    let unit_dir = user_systemd_dir(home);
    std::fs::create_dir_all(&unit_dir)
        .map_err(|error| format!("failed to create {}: {}", unit_dir.display(), error))?;

    std::fs::write(
        unit_dir.join(SERVICE_NAME),
        service_content(executable, threshold, threshold_step),
    )
    .map_err(|error| format!("failed to write {}: {}", SERVICE_NAME, error))?;
    std::fs::write(
        unit_dir.join(TIMER_NAME),
        timer_content(timer_boot_delay, timer_interval),
    )
    .map_err(|error| format!("failed to write {}: {}", TIMER_NAME, error))?;
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let home = PathBuf::from(std::env::var("HOME").map_err(|_| "HOME not set".to_string())?);
    systemctl(&["disable", "--now", TIMER_NAME], true)?;
    systemctl(&["stop", SERVICE_NAME], true)?;

    let unit_dir = user_systemd_dir(&home);
    for name in [SERVICE_NAME, TIMER_NAME] {
        let path = unit_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => println!("Removed {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("failed to remove {}: {}", path.display(), error)),
        }
    }
    systemctl(&["daemon-reload"], false)?;
    systemctl(&["reset-failed", SERVICE_NAME], true)?;
    println!("Uninstalled systemd user service and timer");
    Ok(())
}

fn logs_command(follow: bool) -> Command {
    let mut command = Command::new("journalctl");
    command.arg("--user").arg("--unit").arg(SERVICE_NAME);
    if follow {
        command.arg("--follow");
    }
    command
}

pub fn logs(follow: bool) -> Result<(), String> {
    run(
        &mut logs_command(follow),
        if follow {
            "follow systemd service logs"
        } else {
            "show systemd service logs"
        },
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        install_files, installation_lines, logs_command, quote_systemd, run, service_content,
        timer_content, timer_value_from_env, user_systemd_dir, validate_timer_value,
    };

    #[test]
    fn quotes_systemd_paths_and_runs_commands() {
        assert_eq!(
            quote_systemd(std::path::Path::new(r#"/tmp/a%/b\"c"#)),
            "\"/tmp/a%%/b\\\\\\\"c\""
        );
        assert!(run(&mut std::process::Command::new("true"), "succeed", false).is_ok());
        assert!(run(&mut std::process::Command::new("false"), "fail", true).is_ok());
        assert!(run(&mut std::process::Command::new("false"), "fail", false).is_err());
        assert!(run(
            &mut std::process::Command::new("codex-switch-command-that-does-not-exist"),
            "spawn",
            false,
        )
        .unwrap_err()
        .contains("failed to spawn"));
    }

    #[test]
    fn reads_timer_values_from_environment() {
        let name = "CODEX_SWITCH_TEST_TIMER_VALUE";
        std::env::remove_var(name);
        assert_eq!(timer_value_from_env(name, "5min").unwrap(), "5min");
        std::env::set_var(name, " 30s ");
        assert_eq!(timer_value_from_env(name, "5min").unwrap(), "30s");
        std::env::set_var(name, "bad-value");
        assert!(timer_value_from_env(name, "5min").is_err());
        std::env::remove_var(name);
    }

    #[test]
    fn reports_partial_and_unreadable_installations() {
        let base = std::env::temp_dir().join(format!(
            "codex-switch-systemd-partial-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let unit_dir = user_systemd_dir(&base);
        std::fs::create_dir_all(&unit_dir).unwrap();

        std::fs::write(unit_dir.join("codex-switch.service"), "[Service]\n").unwrap();
        let lines = installation_lines(&base).unwrap().unwrap();
        assert!(lines.iter().any(|line| line.contains("timer: missing")));

        std::fs::remove_file(unit_dir.join("codex-switch.service")).unwrap();
        std::fs::write(unit_dir.join("codex-switch.timer"), "[Timer]\n").unwrap();
        let lines = installation_lines(&base).unwrap().unwrap();
        assert!(lines.iter().any(|line| line.contains("service: missing")));

        std::fs::remove_file(unit_dir.join("codex-switch.timer")).unwrap();
        std::fs::create_dir(unit_dir.join("codex-switch.service")).unwrap();
        assert!(installation_lines(&base).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn service_runs_requested_binary_with_inline_threshold_environment() {
        let content = service_content(
            std::path::Path::new("/tmp/release bin/codex-switch"),
            75.0,
            5.0,
        );
        assert!(content.contains("ExecStart=\"/tmp/release bin/codex-switch\" auto run"));
        assert!(!content.contains("--auto-switch"));
        assert!(content.contains("Environment=CODEX_SWITCH_THRESHOLD_PERCENT=75"));
        assert!(content.contains("Environment=CODEX_SWITCH_THRESHOLD_STEP_PERCENT=5"));
        assert!(!content.contains("EnvironmentFile"));
        assert!(content.contains("Type=oneshot"));
    }

    #[test]
    fn logs_follow_is_optional() {
        let plain = logs_command(false);
        let follow = logs_command(true);
        let plain: Vec<_> = plain.get_args().map(|arg| arg.to_string_lossy()).collect();
        let follow: Vec<_> = follow.get_args().map(|arg| arg.to_string_lossy()).collect();
        assert!(!plain.iter().any(|arg| arg == "--follow"));
        assert!(follow.iter().any(|arg| arg == "--follow"));
    }

    #[test]
    fn install_files_writes_service_and_timer_only() {
        let base =
            std::env::temp_dir().join(format!("codex-switch-systemd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        install_files(
            &base,
            std::path::Path::new("/tmp/codex-switch"),
            82.5,
            2.5,
            "30s",
            "10min",
        )
        .unwrap();

        let service = user_systemd_dir(&base).join("codex-switch.service");
        assert!(service.exists());
        let timer = user_systemd_dir(&base).join("codex-switch.timer");
        assert!(timer.exists());
        let timer = std::fs::read_to_string(timer).unwrap();
        assert!(timer.contains("OnBootSec=30s"));
        assert!(timer.contains("OnUnitActiveSec=10min"));
        let service = std::fs::read_to_string(service).unwrap();
        assert!(service.contains("Environment=CODEX_SWITCH_THRESHOLD_PERCENT=82.5"));
        assert!(service.contains("Environment=CODEX_SWITCH_THRESHOLD_STEP_PERCENT=2.5"));
        assert!(!base.join(".config/codex-switch/auto-switch.env").exists());

        let status = installation_lines(&base).unwrap().unwrap();
        assert!(status
            .iter()
            .any(|line| line.contains("service: installed")));
        assert!(status.iter().any(|line| line == "  threshold: 82.5%"));
        assert!(status.iter().any(|line| line == "  threshold step: 2.5%"));
        assert!(status.iter().any(|line| line.contains("timer: installed")));
        assert!(status.iter().any(|line| line == "  boot delay: 30s"));
        assert!(status.iter().any(|line| line == "  interval: 10min"));
        assert!(status.iter().any(|line| line == "  persistent: true"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn installation_status_is_absent_without_unit_files() {
        let base = std::env::temp_dir().join(format!(
            "codex-switch-systemd-status-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        assert!(installation_lines(&base).unwrap().is_none());
    }

    #[test]
    fn validates_safe_systemd_timer_durations() {
        for value in ["30s", "5min", "1h 30min", "0.5h"] {
            assert_eq!(validate_timer_value("TEST_TIMER", value).unwrap(), value);
        }
        for value in ["", "5min\nOnCalendar=daily", "${VALUE}", "-1min"] {
            assert!(validate_timer_value("TEST_TIMER", value).is_err());
        }
    }

    #[test]
    fn timer_content_uses_configured_schedule_and_remains_persistent() {
        let timer = timer_content("45s", "12min");
        assert!(timer.contains("OnBootSec=45s"));
        assert!(timer.contains("OnUnitActiveSec=12min"));
        assert!(timer.contains("Persistent=true"));
    }
}

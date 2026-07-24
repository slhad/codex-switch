use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "codex-switch",
    version,
    about = "Switch between Codex auth profiles"
)]
pub struct Cli {
    /// Stop running Codex desktop instances and the remote app server before switching
    #[arg(short = 'k', long = "kill", conflicts_with_all = ["install_link", "remove_link", "import_profile", "stop_remote"])]
    pub kill: bool,

    /// Stop the Codex remote app server that owns the control socket
    #[arg(
        long = "stop-remote",
        conflicts_with_all = [
            "kill",
            "debug_usage",
            "debug_pi_usage",
            "codex",
            "pi",
            "install_link",
            "remove_link",
            "restore_last",
            "storage",
            "waybar",
            "install_waybar_config",
            "save_profile",
            "transfer_profile",
            "import_profile",
            "profile"
        ]
    )]
    pub stop_remote: bool,

    /// Print the raw OAuth usage payload when showing status
    #[arg(long = "debug-usage", conflicts_with_all = ["install_link", "remove_link", "import_profile"])]
    pub debug_usage: bool,

    /// Print the raw PI agent OAuth usage payload when showing status
    #[arg(long = "debug-pi-usage", conflicts_with_all = ["install_link", "remove_link", "import_profile"])]
    pub debug_pi_usage: bool,

    /// Switch only the Codex auth profile
    #[arg(
        long = "codex",
        requires = "profile",
        conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "save_profile", "transfer_profile", "import_profile", "pi"]
    )]
    pub codex: bool,

    /// Switch only the PI auth profile
    #[arg(
        long = "pi",
        requires = "profile",
        conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "save_profile", "transfer_profile", "import_profile", "codex"]
    )]
    pub pi: bool,

    /// Switch even if current auth doesn't match a known profile
    #[arg(long = "force", conflicts_with_all = ["install_link", "remove_link"])]
    pub force: bool,

    /// Install a codex-switch symlink into ~/bin or ~/.local/bin
    #[arg(long = "install-link", conflicts_with = "remove_link")]
    pub install_link: bool,

    /// Remove codex-switch symlinks from ~/bin and ~/.local/bin
    #[arg(long = "remove-link", conflicts_with = "install_link")]
    pub remove_link: bool,

    /// Restore the last tracker-backed auth snapshot into ~/.codex/auth.json
    #[arg(long = "restore-last", conflicts_with_all = ["install_link", "remove_link"])]
    pub restore_last: bool,

    /// Show where codex-switch stores live auth, profiles, and tracker files
    #[arg(long = "storage", conflicts_with_all = ["install_link", "remove_link", "restore_last", "codex", "pi"])]
    pub storage: bool,

    /// Print a Waybar JSON payload for Codex usage
    #[arg(long = "waybar", conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "profile", "save_profile", "transfer_profile"])]
    pub waybar: bool,

    /// Hide reset minutes in Waybar when at least one day remains
    #[arg(
        long = "waybar-hide-minutes-with-days",
        requires = "waybar",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    pub waybar_hide_minutes_with_days: bool,

    /// Hide reset hours in Waybar when at least one day remains
    #[arg(
        long = "waybar-hide-hours-with-days",
        requires = "waybar",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    pub waybar_hide_hours_with_days: bool,

    /// Custom Waybar text format for --waybar
    #[arg(long = "format", requires = "waybar")]
    pub format: Option<String>,

    /// Custom Waybar tooltip format for --waybar
    #[arg(long = "tooltip-format", requires = "waybar")]
    pub tooltip_format: Option<String>,

    /// Install or update codex-switch in the user's Waybar config
    #[arg(long = "install-waybar-config", conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "profile", "save_profile", "transfer_profile", "waybar"])]
    pub install_waybar_config: bool,

    /// Save the current live Codex or PI auth into a named profile
    #[arg(
        long = "save-profile",
        value_names = ["STORE", "NAME"],
        num_args = 2,
        conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "profile", "transfer_profile", "waybar", "install_waybar_config"]
    )]
    pub save_profile: Option<Vec<String>>,

    /// Transfer STORE/PROFILE to STORE/PROFILE; codex-to-pi repeats on switches
    #[arg(
        long = "transfer-profile",
        value_names = ["SOURCE", "TARGET"],
        num_args = 2,
        conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "profile", "save_profile", "import_profile", "waybar", "install_waybar_config"]
    )]
    pub transfer_profile: Option<Vec<String>>,

    /// Import a Codex auth.json into a named profile without activating it
    #[arg(
        long = "import-profile",
        value_names = ["NAME", "AUTH_JSON"],
        num_args = 2,
        conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "profile", "save_profile", "transfer_profile", "waybar", "install_waybar_config", "codex", "pi"]
    )]
    pub import_profile: Option<Vec<String>>,

    /// Profile name to switch to
    #[arg(conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "save_profile", "transfer_profile", "import_profile", "waybar", "install_waybar_config"])]
    pub profile: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parses_stop_remote_as_a_standalone_command() {
        let cli = Cli::try_parse_from(["codex-switch", "--stop-remote"]).unwrap();
        assert!(cli.stop_remote);
        assert!(!cli.kill);
    }

    #[test]
    fn rejects_stop_remote_with_kill_or_profile() {
        assert!(Cli::try_parse_from(["codex-switch", "--stop-remote", "--kill"]).is_err());
        assert!(Cli::try_parse_from(["codex-switch", "--stop-remote", "work"]).is_err());
    }

    #[test]
    fn parses_compact_transfer_profile_references() {
        let cli = Cli::try_parse_from([
            "codex-switch",
            "--transfer-profile",
            "codex/mate",
            "pi/mate",
        ])
        .unwrap();
        assert_eq!(
            cli.transfer_profile.as_deref(),
            Some(["codex/mate".to_string(), "pi/mate".to_string()].as_slice())
        );
        assert!(Cli::try_parse_from([
            "codex-switch",
            "--transfer-profile",
            "codex",
            "mate",
            "pi",
            "mate"
        ])
        .is_err());
    }
}

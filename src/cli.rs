use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "codex-switch",
    version,
    about = "Switch between Codex auth profiles"
)]
pub struct Cli {
    /// Stop running Codex desktop instances before switching
    #[arg(short = 'k', long = "kill", conflicts_with_all = ["install_link", "remove_link"])]
    pub kill: bool,

    /// Print the raw OAuth usage payload when showing status
    #[arg(long = "debug-usage", conflicts_with_all = ["install_link", "remove_link"])]
    pub debug_usage: bool,

    /// Print the raw PI agent OAuth usage payload when showing status
    #[arg(long = "debug-pi-usage", conflicts_with_all = ["install_link", "remove_link"])]
    pub debug_pi_usage: bool,

    /// Switch only the Codex auth profile
    #[arg(
        long = "codex",
        requires = "profile",
        conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "save_profile", "transfer_profile", "pi"]
    )]
    pub codex: bool,

    /// Switch only the PI auth profile
    #[arg(
        long = "pi",
        requires = "profile",
        conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "save_profile", "transfer_profile", "codex"]
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

    /// Transfer a saved profile between codex/pi stores
    #[arg(
        long = "transfer-profile",
        value_names = ["SOURCE_STORE", "SOURCE_NAME", "TARGET_STORE", "TARGET_NAME"],
        num_args = 4,
        conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "profile", "save_profile", "waybar", "install_waybar_config"]
    )]
    pub transfer_profile: Option<Vec<String>>,

    /// Profile name to switch to
    #[arg(conflicts_with_all = ["install_link", "remove_link", "restore_last", "storage", "save_profile", "transfer_profile", "waybar", "install_waybar_config"])]
    pub profile: Option<String>,
}

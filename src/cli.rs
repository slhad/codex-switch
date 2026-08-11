use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Parser, Debug)]
#[command(
    name = "codex-switch",
    version,
    about = "Switch between Codex auth profiles"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Profile name to switch to (shorthand for `switch PROFILE`)
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::all_profiles))]
    pub profile: Option<ProfileName>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show current profile and OAuth usage status
    Status(StatusArgs),
    /// Switch to a saved profile
    Switch(SwitchArgs),
    /// Stop Codex processes
    Stop(StopArgs),
    /// Manage saved profiles and transfers
    Profile(ProfileArgs),
    /// Run and configure automatic switching
    Auto(AutoArgs),
    /// Manage the systemd user service
    Service(ServiceArgs),
    /// Manage the command symlink
    Link(LinkArgs),
    /// Print or install Waybar integration
    Waybar(WaybarArgs),
    /// Manage tracked sessions
    Tracker(TrackerArgs),
    /// Show storage paths
    Storage,
    /// Manage recovery snapshots
    Recovery(RecoveryArgs),
    /// Print or install shell completion
    Completion(CompletionArgs),
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Print raw OAuth usage payloads
    #[arg(long, value_enum)]
    pub debug: Option<DebugTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum DebugTarget {
    Codex,
    Pi,
    All,
}

#[derive(Args, Debug)]
pub struct SwitchArgs {
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::all_profiles))]
    pub profile: ProfileName,
    #[arg(long, value_enum, default_value_t = SwitchTarget::Both)]
    pub target: SwitchTarget,
    #[arg(long)]
    pub force: bool,
    /// Stop desktop and remote processes before switching
    #[arg(long)]
    pub kill: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum SwitchTarget {
    Codex,
    Pi,
    #[default]
    Both,
}

#[derive(Args, Debug)]
pub struct StopArgs {
    /// Stop only the remote app server
    #[arg(long)]
    pub remote_only: bool,
}

#[derive(Args, Debug)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub command: ProfileCommand,
}
#[derive(Subcommand, Debug)]
pub enum ProfileCommand {
    Save(ProfileStoreNameArgs),
    Remove(ProfileRemoveArgs),
    Import(ProfileImportArgs),
    /// Create a T3 Code shadow home from a saved Codex profile
    ShadowHome(ProfileShadowHomeArgs),
    Transfer(ProfileTransferArgs),
}
#[derive(Args, Debug)]
pub struct ProfileStoreNameArgs {
    pub store: ProfileStore,
    pub name: ProfileName,
}
#[derive(Args, Debug)]
pub struct ProfileRemoveArgs {
    pub store: ProfileStore,
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::remove_profiles))]
    pub name: ProfileName,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ProfileStore {
    Codex,
    Pi,
}
impl ProfileStore {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }
}

#[derive(Args, Debug)]
pub struct ProfileImportArgs {
    #[command(subcommand)]
    pub command: ProfileImportCommand,
}
#[derive(Subcommand, Debug)]
pub enum ProfileImportCommand {
    Codex(ProfileImportCodexArgs),
}
#[derive(Args, Debug)]
pub struct ProfileImportCodexArgs {
    pub name: ProfileName,
    #[arg(
        value_hint = clap::ValueHint::FilePath,
        add = clap_complete::ArgValueCompleter::new(clap_complete::PathCompleter::file())
    )]
    pub auth_json: PathBuf,
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct ProfileShadowHomeArgs {
    /// Saved Codex profile to install as the shadow home's private auth
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::codex_profiles))]
    pub profile: ProfileName,
    /// Destination; defaults to ~/.codex-t3/PROFILE
    #[arg(
        long,
        value_hint = clap::ValueHint::DirPath,
        add = clap_complete::ArgValueCompleter::new(clap_complete::PathCompleter::dir())
    )]
    pub path: Option<PathBuf>,
    /// Replace a different existing shadow auth.json
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct ProfileTransferArgs {
    #[command(subcommand)]
    pub command: ProfileTransferCommand,
}
#[derive(Subcommand, Debug)]
pub enum ProfileTransferCommand {
    Now(TransferNowArgs),
    OnSwitch(OnSwitchArgs),
}
#[derive(Args, Debug)]
pub struct TransferNowArgs {
    /// Source profile as STORE/PROFILE
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::profile_references))]
    pub source: ProfileRef,
    /// Target profile as STORE/PROFILE
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::profile_references))]
    pub target: ProfileRef,
}
#[derive(Args, Debug)]
pub struct OnSwitchArgs {
    #[command(subcommand)]
    pub command: OnSwitchCommand,
}
#[derive(Subcommand, Debug)]
pub enum OnSwitchCommand {
    Set(OnSwitchSetArgs),
    Enable(ProfileNameArgs),
    Disable(ProfileNameArgs),
}
#[derive(Args, Debug)]
pub struct OnSwitchSetArgs {
    /// Source Codex profile name (without a store prefix)
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::codex_profiles))]
    pub source: ProfileName,
    /// Target PI profile name (without a store prefix)
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::pi_profiles))]
    pub target: ProfileName,
}
#[derive(Args, Debug)]
pub struct ProfileNameArgs {
    /// Profile name without a store prefix
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::transfer_profiles))]
    pub profile: ProfileName,
}

#[derive(Args, Debug)]
pub struct AutoArgs {
    #[command(subcommand)]
    pub command: AutoCommand,
}
#[derive(Subcommand, Debug)]
pub enum AutoCommand {
    Run(AutoRunArgs),
    Show,
    Set(AutoSetArgs),
    Remove(AutoRemoveArgs),
}
#[derive(Args, Debug)]
pub struct AutoRunArgs {
    #[arg(long)]
    pub dry_run: bool,
}
#[derive(Args, Debug)]
#[command(group(ArgGroup::new("settings").required(true).multiple(true).args(["enabled", "priority", "codex", "pi"])))]
pub struct AutoSetArgs {
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::all_profiles))]
    pub profile: ProfileName,
    #[arg(long)]
    pub enabled: Option<bool>,
    #[arg(long, allow_hyphen_values = true)]
    pub priority: Option<i32>,
    #[arg(long)]
    pub codex: Option<bool>,
    #[arg(long)]
    pub pi: Option<bool>,
}
#[derive(Args, Debug)]
pub struct AutoRemoveArgs {
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::auto_profiles))]
    pub profile: ProfileName,
}

#[derive(Args, Debug)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub command: ServiceCommand,
}
#[derive(Subcommand, Debug)]
pub enum ServiceCommand {
    Install,
    Uninstall,
    Logs(ServiceLogsArgs),
}
#[derive(Args, Debug)]
pub struct ServiceLogsArgs {
    #[arg(long)]
    pub follow: bool,
}

#[derive(Args, Debug)]
pub struct LinkArgs {
    #[command(subcommand)]
    pub command: LinkCommand,
}
#[derive(Subcommand, Debug)]
pub enum LinkCommand {
    Install,
    Uninstall,
}

#[derive(Args, Debug)]
pub struct WaybarArgs {
    #[command(subcommand)]
    pub command: WaybarCommand,
}
#[derive(Subcommand, Debug)]
pub enum WaybarCommand {
    Print(WaybarPrintArgs),
    Install,
}
#[derive(Args, Debug)]
pub struct WaybarPrintArgs {
    #[arg(long)]
    pub format: Option<String>,
    #[arg(long)]
    pub tooltip_format: Option<String>,
    /// Display remaining percentages instead of used percentages
    #[arg(long)]
    pub percent_left: bool,
    #[arg(long = "waybar-hide-minutes-with-days", default_value_t = true, action = clap::ArgAction::Set)]
    pub hide_minutes_with_days: bool,
    #[arg(long = "waybar-hide-hours-with-days", default_value_t = true, action = clap::ArgAction::Set)]
    pub hide_hours_with_days: bool,
}

#[derive(Args, Debug)]
pub struct TrackerArgs {
    #[command(subcommand)]
    pub command: TrackerCommand,
}
#[derive(Subcommand, Debug)]
pub enum TrackerCommand {
    /// List tracked sessions and their exact IDs
    List,
    Remove(TrackerRemoveArgs),
}
#[derive(Args, Debug)]
pub struct TrackerRemoveArgs {
    pub session_id: String,
}

#[derive(Args, Debug)]
pub struct RecoveryArgs {
    #[command(subcommand)]
    pub command: RecoveryCommand,
}
#[derive(Subcommand, Debug)]
pub enum RecoveryCommand {
    Restore,
}

#[derive(Args, Debug)]
pub struct CompletionArgs {
    #[command(subcommand)]
    pub command: CompletionCommand,
}
#[derive(Subcommand, Debug)]
pub enum CompletionCommand {
    /// Print a Bash loader that enables dynamic completion
    Bash,
    /// Install Bash completion in the per-user XDG data directory
    Install(CompletionInstallArgs),
}
#[derive(Args, Debug)]
pub struct CompletionInstallArgs {
    /// Replace an existing completion file with different content
    #[arg(long)]
    pub force: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileName(String);
impl ProfileName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for ProfileName {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        crate::switch::validate_profile_name(value)?;
        Ok(Self(value.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRef {
    pub store: ProfileStore,
    pub profile: ProfileName,
}
impl ProfileRef {
    pub fn compact(&self) -> String {
        format!("{}/{}", self.store.as_str(), self.profile.as_str())
    }
}
impl FromStr for ProfileRef {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (store, profile) = value
            .split_once('/')
            .ok_or_else(|| "profile reference must use STORE/PROFILE".to_string())?;
        if profile.contains('/') {
            return Err("profile reference must use STORE/PROFILE".to_string());
        }
        let store = match store {
            "codex" => ProfileStore::Codex,
            "pi" => ProfileStore::Pi,
            _ => return Err("profile store must be `codex` or `pi`".to_string()),
        };
        Ok(Self {
            store,
            profile: profile.parse()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn parses_approved_leaf_commands() {
        let accepted = [
            vec!["codex-switch"],
            vec!["codex-switch", "me"],
            vec!["codex-switch", "status"],
            vec!["codex-switch", "status", "--debug", "all"],
            vec!["codex-switch", "switch", "work"],
            vec![
                "codex-switch",
                "switch",
                "work",
                "--target",
                "pi",
                "--force",
                "--kill",
            ],
            vec!["codex-switch", "stop"],
            vec!["codex-switch", "stop", "--remote-only"],
            vec!["codex-switch", "profile", "save", "codex", "work"],
            vec!["codex-switch", "profile", "remove", "pi", "work"],
            vec![
                "codex-switch",
                "profile",
                "import",
                "codex",
                "work",
                "auth.json",
                "--force",
            ],
            vec![
                "codex-switch",
                "profile",
                "shadow-home",
                "work",
                "--path",
                "/tmp/codex-work",
                "--force",
            ],
            vec![
                "codex-switch",
                "profile",
                "transfer",
                "now",
                "codex/work",
                "pi/work",
            ],
            vec![
                "codex-switch",
                "profile",
                "transfer",
                "on-switch",
                "set",
                "work",
                "work-pi",
            ],
            vec![
                "codex-switch",
                "profile",
                "transfer",
                "on-switch",
                "enable",
                "work",
            ],
            vec![
                "codex-switch",
                "profile",
                "transfer",
                "on-switch",
                "disable",
                "work",
            ],
            vec!["codex-switch", "auto", "run", "--dry-run"],
            vec!["codex-switch", "auto", "show"],
            vec![
                "codex-switch",
                "auto",
                "set",
                "work",
                "--enabled",
                "false",
                "--priority",
                "-2",
                "--codex",
                "true",
                "--pi",
                "false",
            ],
            vec!["codex-switch", "auto", "remove", "work"],
            vec!["codex-switch", "service", "install"],
            vec!["codex-switch", "service", "uninstall"],
            vec!["codex-switch", "service", "logs"],
            vec!["codex-switch", "service", "logs", "--follow"],
            vec!["codex-switch", "link", "install"],
            vec!["codex-switch", "link", "uninstall"],
            vec![
                "codex-switch",
                "waybar",
                "print",
                "--format",
                "{profile}",
                "--tooltip-format",
                "x",
                "--percent-left",
                "--waybar-hide-minutes-with-days",
                "false",
                "--waybar-hide-hours-with-days",
                "false",
            ],
            vec!["codex-switch", "waybar", "install"],
            vec!["codex-switch", "tracker", "list"],
            vec!["codex-switch", "tracker", "remove", "session"],
            vec!["codex-switch", "storage"],
            vec!["codex-switch", "recovery", "restore"],
            vec!["codex-switch", "completion", "bash"],
            vec!["codex-switch", "completion", "install"],
            vec!["codex-switch", "completion", "install", "--force"],
        ];
        for args in accepted {
            assert!(Cli::try_parse_from(&args).is_ok(), "rejected {args:?}");
        }
    }

    #[test]
    fn parses_waybar_percent_left_mode() {
        let cli =
            Cli::try_parse_from(["codex-switch", "waybar", "print", "--percent-left"]).unwrap();
        let Some(Command::Waybar(args)) = cli.command else {
            panic!("expected waybar command");
        };
        let WaybarCommand::Print(args) = args.command else {
            panic!("expected waybar print command");
        };
        assert!(args.percent_left);

        let cli = Cli::try_parse_from(["codex-switch", "waybar", "print"]).unwrap();
        let Some(Command::Waybar(args)) = cli.command else {
            panic!("expected waybar command");
        };
        let WaybarCommand::Print(args) = args.command else {
            panic!("expected waybar print command");
        };
        assert!(!args.percent_left);
    }

    #[test]
    fn validates_defaults_and_typed_values() {
        let Cli {
            command: Some(Command::Switch(args)),
            ..
        } = Cli::try_parse_from(["codex-switch", "switch", "work"]).unwrap()
        else {
            panic!()
        };
        assert_eq!(args.target, SwitchTarget::Both);

        let shorthand = Cli::try_parse_from(["codex-switch", "me"]).unwrap();
        assert!(shorthand.command.is_none());
        assert_eq!(shorthand.profile.unwrap().as_str(), "me");

        let reserved = Cli::try_parse_from(["codex-switch", "status"]).unwrap();
        assert!(matches!(reserved.command, Some(Command::Status(_))));
        assert!(reserved.profile.is_none());
        assert!(Cli::try_parse_from(["codex-switch", "auto", "set", "work"]).is_err());
        assert!(
            Cli::try_parse_from(["codex-switch", "auto", "set", "work", "--pi", "maybe"]).is_err()
        );
        assert!(Cli::try_parse_from([
            "codex-switch",
            "profile",
            "transfer",
            "now",
            "bad",
            "pi/work"
        ])
        .is_err());
        assert!(Cli::try_parse_from(["codex-switch", "switch", "bad/name"]).is_err());
    }

    #[test]
    fn profile_arguments_have_dynamic_completers() {
        fn command_at<'a>(mut command: &'a clap::Command, path: &[&str]) -> &'a clap::Command {
            for name in path {
                command = command.find_subcommand(name).unwrap();
            }
            command
        }
        fn assert_candidates(command: &clap::Command, path: &[&str], argument: &str) {
            let argument = command_at(command, path)
                .get_arguments()
                .find(|arg| arg.get_id() == argument)
                .unwrap();
            assert!(
                argument
                    .get::<clap_complete::ArgValueCandidates>()
                    .is_some(),
                "missing candidates for {path:?} {argument:?}"
            );
        }

        let command = Cli::command();
        for (path, argument) in [
            (&[][..], "profile"),
            (&["switch"][..], "profile"),
            (&["auto", "set"][..], "profile"),
            (&["auto", "remove"][..], "profile"),
            (&["profile", "remove"][..], "name"),
            (&["profile", "shadow-home"][..], "profile"),
            (&["profile", "transfer", "now"][..], "source"),
            (&["profile", "transfer", "now"][..], "target"),
            (&["profile", "transfer", "on-switch", "set"][..], "source"),
            (&["profile", "transfer", "on-switch", "set"][..], "target"),
            (
                &["profile", "transfer", "on-switch", "enable"][..],
                "profile",
            ),
            (
                &["profile", "transfer", "on-switch", "disable"][..],
                "profile",
            ),
        ] {
            assert_candidates(&command, path, argument);
        }

        let auth_json = command_at(&command, &["profile", "import", "codex"])
            .get_arguments()
            .find(|arg| arg.get_id() == "auth_json")
            .unwrap();
        assert!(auth_json
            .get::<clap_complete::ArgValueCompleter>()
            .is_some());
    }

    #[test]
    fn rejects_legacy_surface() {
        for args in [
            vec!["codex-switch", "-k"],
            vec!["codex-switch", "--kill"],
            vec!["codex-switch", "--auto-switch"],
            vec!["codex-switch", "--waybar"],
            vec!["codex-switch", "--debug-usage"],
            vec!["codex-switch", "--debug-pi-usage"],
            vec!["codex-switch", "--install-waybar-config"],
            vec!["codex-switch", "--install-service"],
            vec!["codex-switch", "--save-profile", "codex", "work"],
            vec![
                "codex-switch",
                "--transfer-profile",
                "codex/work",
                "pi/work",
            ],
            vec!["codex-switch", "--storage"],
            vec!["codex-switch", "--remove-link"],
        ] {
            assert!(
                Cli::try_parse_from(&args).is_err(),
                "accepted legacy {args:?}"
            );
        }
    }
}

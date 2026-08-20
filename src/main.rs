mod auto_switch;
mod cli;
mod completions;
mod data;
mod install;
mod jwt;
mod omarchy;
mod process;
mod profile;
mod profile_options;
mod rate_limit;
mod status;
mod storage;
mod switch;
mod systemd;
mod t3_code;
mod tracker;
mod waybar;
mod waybar_config;

use clap::{CommandFactory, Parser};
use cli::*;

fn main() {
    clap_complete::CompleteEnv::with_factory(Cli::command)
        .var("CODEX_SWITCH_COMPLETE")
        .complete();

    let Cli { command, profile } = Cli::parse();
    match command {
        Some(Command::Completion(args)) => run_completion(args),
        command => {
            let ctx = data::Context::new();
            dispatch(command, profile, &ctx);
        }
    }
}

fn dispatch(command: Option<Command>, profile: Option<ProfileName>, ctx: &data::Context) {
    match command {
        None => {
            if let Some(profile) = profile {
                switch::switch_profile(ctx, profile.as_str(), false, switch::SwitchScope::Both);
            } else {
                status::show_status(ctx, false, false);
            }
        }
        Some(Command::Status(args)) => {
            let (codex, pi) = match args.debug {
                None => (false, false),
                Some(DebugTarget::Codex) => (true, false),
                Some(DebugTarget::Pi) => (false, true),
                Some(DebugTarget::All) => (true, true),
            };
            status::show_status(ctx, codex, pi);
        }
        Some(Command::Switch(args)) => {
            if args.kill {
                process::kill_codex_desktop(ctx);
                process::stop_codex_remote(ctx).unwrap_or_else(|error| data::die(&error));
            }
            let scope = match args.target {
                SwitchTarget::Codex => switch::SwitchScope::CodexOnly,
                SwitchTarget::Pi => switch::SwitchScope::PiOnly,
                SwitchTarget::Both => switch::SwitchScope::Both,
            };
            switch::switch_profile(ctx, args.profile.as_str(), args.force, scope);
        }
        Some(Command::Stop(args)) => {
            if !args.remote_only {
                process::kill_codex_desktop(ctx);
            }
            process::stop_codex_remote(ctx).unwrap_or_else(|error| data::die(&error));
        }
        Some(Command::Profile(args)) => match args.command {
            ProfileCommand::Save(args) => {
                switch::save_profile(ctx, args.store.as_str(), args.name.as_str())
            }
            ProfileCommand::Remove(args) => {
                let path = profile::remove_profile(ctx, args.store.as_str(), args.name.as_str())
                    .unwrap_or_else(|error| data::die(&error));
                println!(
                    "Removed saved {} profile: {}",
                    args.store.as_str(),
                    path.display()
                );
            }
            ProfileCommand::Import(args) => match args.command {
                ProfileImportCommand::Codex(args) => {
                    switch::import_profile(ctx, args.name.as_str(), &args.auth_json, args.force)
                }
            },
            ProfileCommand::ShadowHome(args) => {
                let result = t3_code::bootstrap_shadow_home(
                    ctx,
                    args.profile.as_str(),
                    args.path.as_deref(),
                    args.force,
                )
                .unwrap_or_else(|error| data::die(&error));
                println!(
                    "T3 Code shadow home {}:\n  profile: {}\n  shared CODEX_HOME: {}\n  shadow home: {}\n  private auth: {}\n  tracked profile link: {}\nLive Codex auth was not changed: {}\n\nT3 Code provider settings:\n  Binary path: codex\n  CODEX_HOME path: {}\n  Shadow home path: {}",
                    if result.changed { "created" } else { "already up to date" },
                    args.profile.as_str(),
                    result.shared_home.display(),
                    result.shadow_home.display(),
                    result.auth_path.display(),
                    result.profile_path.display(),
                    ctx.live_auth.display(),
                    result.shared_home.display(),
                    result.shadow_home.display(),
                );
            }
            ProfileCommand::Transfer(args) => match args.command {
                ProfileTransferCommand::Now(args) => {
                    switch::transfer_profile(ctx, &args.source.compact(), &args.target.compact())
                }
                ProfileTransferCommand::OnSwitch(args) => match args.command {
                    OnSwitchCommand::Set(args) => {
                        profile_options::configure_transfer(
                            ctx,
                            args.source.as_str(),
                            args.target.as_str(),
                        )
                        .unwrap_or_else(|error| data::die(&error));
                        println!(
                            "Transfer on switch configured: codex/{} -> pi/{} (enabled)",
                            args.source.as_str(),
                            args.target.as_str()
                        );
                    }
                    OnSwitchCommand::Enable(args) => set_transfer_enabled(ctx, &args.profile, true),
                    OnSwitchCommand::Disable(args) => {
                        set_transfer_enabled(ctx, &args.profile, false)
                    }
                },
            },
        },
        Some(Command::Auto(args)) => match args.command {
            AutoCommand::Run(args) => {
                auto_switch::run(ctx, args.dry_run).unwrap_or_else(|error| data::die(&error))
            }
            AutoCommand::Show => {
                auto_switch::show_config(ctx).unwrap_or_else(|error| data::die(&error))
            }
            AutoCommand::Set(args) => auto_switch::set_profile_policy(
                ctx,
                args.profile.as_str(),
                args.enabled,
                args.priority,
                args.codex,
                args.pi,
            )
            .unwrap_or_else(|error| data::die(&error)),
            AutoCommand::Remove(args) => {
                auto_switch::remove_profile_policy(ctx, args.profile.as_str())
                    .unwrap_or_else(|error| data::die(&error))
            }
        },
        Some(Command::Service(args)) => match args.command {
            ServiceCommand::Install => systemd::install().unwrap_or_else(|error| data::die(&error)),
            ServiceCommand::Uninstall => {
                systemd::uninstall().unwrap_or_else(|error| data::die(&error))
            }
            ServiceCommand::Logs(args) => {
                systemd::logs(args.follow).unwrap_or_else(|error| data::die(&error))
            }
        },
        Some(Command::Link(args)) => match args.command {
            LinkCommand::Install => install::install_link(),
            LinkCommand::Uninstall => install::remove_link(),
        },
        Some(Command::Waybar(args)) => match args.command {
            WaybarCommand::Print(args) => waybar::print_waybar(
                ctx,
                args.format.as_deref(),
                args.tooltip_format.as_deref(),
                args.hide_minutes_with_days,
                args.hide_hours_with_days,
                args.percent_left,
            ),
            WaybarCommand::Install => waybar_config::install_waybar_config(),
        },
        Some(Command::Omarchy(args)) => match args.command {
            OmarchyCommand::Print => omarchy::print_snapshot(ctx),
        },
        Some(Command::Tracker(args)) => match args.command {
            TrackerCommand::List => println!(
                "{}",
                tracker::list_sessions(ctx).unwrap_or_else(|error| data::die(&error))
            ),
            TrackerCommand::Remove(args) => {
                let removed = tracker::remove_session(ctx, &args.session_id)
                    .unwrap_or_else(|error| data::die(&error));
                if !removed {
                    data::die(&format!(
                        "tracked session `{}` does not exist",
                        args.session_id
                    ));
                }
                println!("Removed tracked session: {}", args.session_id);
            }
        },
        Some(Command::Storage) => storage::show_storage(ctx),
        Some(Command::Recovery(args)) => match args.command {
            RecoveryCommand::Restore => switch::restore_last(ctx),
        },
        Some(Command::Completion(_)) => unreachable!("completion is dispatched before Context"),
    }
}

fn run_completion(args: CompletionArgs) {
    match args.command {
        CompletionCommand::Bash => print!("{}", completions::bash_script()),
        CompletionCommand::Install(args) => {
            let path =
                completions::default_completion_path().unwrap_or_else(|error| data::die(&error));
            let changed = completions::install_bash(&path, args.force)
                .unwrap_or_else(|error| data::die(&error));
            if changed {
                println!("Installed Bash completion: {}", path.display());
            } else {
                println!("Bash completion already up to date: {}", path.display());
            }
            println!("Start a new Bash shell or source {}", path.display());
        }
    }
}

fn set_transfer_enabled(ctx: &data::Context, profile: &ProfileName, enabled: bool) {
    let target = profile_options::set_transfer_enabled(ctx, profile.as_str(), enabled)
        .unwrap_or_else(|error| data::die(&error));
    println!(
        "Transfer on switch: codex/{} -> pi/{} enabled={}",
        profile.as_str(),
        target,
        enabled
    );
}

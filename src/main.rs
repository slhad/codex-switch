mod cli;
mod data;
mod install;
mod jwt;
mod process;
mod profile;
mod rate_limit;
mod status;
mod storage;
mod switch;
mod tracker;
mod waybar;
mod waybar_config;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    let ctx = data::Context::new();

    if cli.install_link {
        install::install_link();
        return;
    }

    if cli.remove_link {
        install::remove_link();
        return;
    }

    if cli.restore_last {
        switch::restore_last(&ctx);
        return;
    }

    if cli.storage {
        storage::show_storage(&ctx);
        return;
    }

    if cli.waybar {
        waybar::print_waybar(&ctx, cli.format.as_deref(), cli.tooltip_format.as_deref());
        return;
    }

    if cli.install_waybar_config {
        waybar_config::install_waybar_config();
        return;
    }

    if let Some(args) = cli.save_profile.as_ref() {
        switch::save_profile(&ctx, &args[0], &args[1]);
        return;
    }

    if let Some(args) = cli.transfer_profile.as_ref() {
        switch::transfer_profile(&ctx, &args[0], &args[1], &args[2], &args[3]);
        return;
    }

    if cli.kill {
        process::kill_codex_desktop(&ctx);
        if cli.profile.is_none() {
            return;
        }
    }

    if let Some(target) = cli.profile {
        let scope = if cli.codex {
            switch::SwitchScope::CodexOnly
        } else if cli.pi {
            switch::SwitchScope::PiOnly
        } else {
            switch::SwitchScope::Both
        };
        switch::switch_profile(&ctx, &target, cli.force, scope);
    } else {
        status::show_status(&ctx, cli.debug_usage, cli.debug_pi_usage);
    }
}

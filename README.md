# codex-switch

Rust CLI for switching saved Codex and PI OAuth profiles and inspecting OAuth-backed usage.

## Overview

This project is OAuth-only. It reads live credentials from `~/.codex/auth.json` and `~/.pi/agent/auth.json`, derives OAuth client IDs from available JWT claims, and queries the ChatGPT usage endpoint. Saved profiles, automatic-switch policies, transfer mappings, and tracker state live under `~/.local/state/codex-switch/`.

Before a switch, the current Codex auth is stored as a recovery snapshot. Saved Codex and PI profiles remain separate from live auth. Profile names may contain only ASCII letters, digits, `_`, and `-`.

Automatic switching runs only when the current profile has an enabled automatic-switch policy for the relevant Codex or PI store, then evaluates the highest reported consumption across applicable windows. Its default threshold is 90%, with a 5% selection step. Codex and PI candidates are selected independently by eligibility, priority, usage, and name. `profile transfer now codex/NAME pi/NAME` performs an immediate transfer and remembers an enabled mapping for later normal switches.

## Build and test

Use the local `rtk` wrapper:

```bash
rtk cargo fmt
rtk cargo test
rtk scripts/check-coverage.sh
rtk cargo build --release
```

The release binary is `target/release/codex-switch`.

## CLI

Running with no arguments is equivalent to `status`. A bare profile name is shorthand for switching both Codex and PI; profile names that match a command name must use the explicit `switch` form.

```text
codex-switch
codex-switch PROFILE
codex-switch status [--debug codex|pi|all]
codex-switch switch PROFILE [--target codex|pi|both] [--force] [--kill]
codex-switch stop [--remote-only]

codex-switch profile save STORE NAME
codex-switch profile remove STORE NAME
codex-switch profile import codex NAME AUTH_JSON [--force]
codex-switch profile shadow-home PROFILE [--path DIRECTORY] [--force]
codex-switch profile transfer now SOURCE TARGET
codex-switch profile transfer on-switch set SOURCE TARGET
codex-switch profile transfer on-switch enable SOURCE
codex-switch profile transfer on-switch disable SOURCE

codex-switch auto run [--dry-run]
codex-switch auto show
codex-switch auto set PROFILE [--enabled BOOL] [--priority NUMBER] [--codex BOOL] [--pi BOOL]
codex-switch auto remove PROFILE

codex-switch service install
codex-switch service uninstall
codex-switch service logs [--follow]
codex-switch link install
codex-switch link uninstall

codex-switch waybar print [--format FORMAT] [--tooltip-format FORMAT]
    [--percent-left]
    [--waybar-hide-minutes-with-days BOOL]
    [--waybar-hide-hours-with-days BOOL]
codex-switch waybar install

codex-switch tracker list
codex-switch tracker remove SESSION_ID
codex-switch storage
codex-switch recovery restore

codex-switch completion bash
codex-switch completion install [--force]
```

`STORE` is `codex` or `pi`. The immediate `profile transfer now` operands use exact `STORE/PROFILE` references. On-switch mappings always transfer Codex to PI, so `set`, `enable`, and `disable` take bare profile names. Bare `codex-switch` lists every saved Codex profile in the `Profile options` section and shows its automatic-switch policy plus whether PI transfer is enabled, disabled, or not configured (`transfer none`). `switch` defaults to `--target both`. `auto set` requires at least one setting and accepts multiple settings atomically. Plain `service logs` prints the journal and exits; `--follow` continues following it. Use `tracker list` to print every tracked session's exact `SESSION_ID` before removing one.

Examples:

```bash
rtk cargo run -- status --debug all
rtk cargo run -- switch work --target codex
rtk cargo run -- profile save codex work
rtk cargo run -- profile import codex work ./auth.json
rtk cargo run -- profile shadow-home work
rtk cargo run -- profile transfer now codex/work pi/work
rtk cargo run -- profile transfer on-switch disable work
rtk cargo run -- auto set work --priority 100 --pi false
rtk cargo run -- auto run --dry-run
rtk cargo run -- service logs
rtk cargo run -- waybar print
rtk cargo run -- storage
```

## T3 Code shadow homes

`profile shadow-home PROFILE` creates a T3 Code authentication overlay from a saved Codex profile. By default it writes the private account credential to `~/.codex-t3/PROFILE/auth.json`, links the saved profile to that private file so token refreshes and usage tracking remain synchronized, and prints the shared `CODEX_HOME` and shadow-home values to enter in T3 Code. It does not modify `~/.codex/auth.json`, so the currently active personal account remains unchanged while the T3 provider stays pinned to the selected profile.

The command is idempotent when the shadow auth and saved-profile link already match. It refuses to replace a different `auth.json` without `--force`, rejects symlinked shadow auth files and saved profiles linked elsewhere, and preserves the saved profile's file permissions. Use `--path DIRECTORY` to choose another location. T3 Code materializes the shared session, plugin, skill, and configuration links when it starts the provider.

## Bash completion

Enable completion for the current shell without installing a file:

```bash
source <(codex-switch completion bash)
```

Install it for future Bash sessions:

```bash
codex-switch completion install
```

The default destination is `${XDG_DATA_HOME:-$HOME/.local/share}/bash-completion/completions/codex-switch`. A relative `XDG_DATA_HOME` is ignored and falls back to the absolute `HOME` path. Parent directories are created as needed. Reinstalling identical content is a no-op; a different existing file is atomically preserved unless `--force` is supplied. The installed loader asks the running `codex-switch` binary for completions, so saved Codex and PI profile names update dynamically without reinstalling. Profile removal candidates follow its `STORE`; profile import keeps normal filename completion for `AUTH_JSON`.

## Automatic switching service

`service install` writes and enables a systemd user oneshot service and timer. The generated service runs:

```text
codex-switch auto run
```

Configuration environment variables:

- `CODEX_SWITCH_THRESHOLD_PERCENT` (default `90`)
- `CODEX_SWITCH_THRESHOLD_STEP_PERCENT` (default `5`)
- `CODEX_SWITCH_TIMER_BOOT_DELAY` (default `2min`)
- `CODEX_SWITCH_TIMER_INTERVAL` (default `5min`)

For example:

```bash
CODEX_SWITCH_THRESHOLD_PERCENT=85 \
CODEX_SWITCH_TIMER_INTERVAL=10min \
  target/release/codex-switch service install
```

Service uninstall preserves profile policies.

## Waybar

`waybar print` emits JSON with the active profile in `text` and saved profile details in `tooltip` and `alt`. By default, personal subscriptions and Business/Enterprise monthly spend controls display the percentage used. Pass `--percent-left` to display the percentage remaining for every usage window; the output stays compact and does not add a `left` label. `waybar install` adds or updates `custom/codex-usage` while preserving unrelated settings and writing a timestamped backup before editing. Generated configuration invokes `codex-switch waybar print`.

Format tokens include `{usage_block}`, `{usage_block_pango}`, `{icon}`, `{time_icon}`, `{5h_pct}`, `{7d_pct}`, `{5h_used_pct}`, `{5h_remaining_pct}`, `{7d_used_pct}`, `{7d_remaining_pct}`, `{monthly_pct}`, `{monthly_used_pct}`, `{monthly_remaining_pct}`, `{available_resets}`, `{applicable_resets}`, `{reset_expiry}`, `{reset_expiry_at}`, `{status}`, `{profile}`, `{provider}`, `{email}`, `{pct}`, `{reset}`, and `{win}`. `{5h_pct}`, `{7d_pct}`, `{monthly_pct}`, and `{pct}` follow the selected percentage mode; explicitly named used/remaining tokens do not. The default tooltip lists each available reset credit and its expiration when the reset-credit API provides details.

## PI OAuth hot-reload extension

This repository is also a PI package. Its extension wraps PI's public `openai-codex` provider authentication and reads the current OAuth credential from `~/.pi/agent/auth.json` whenever PI resolves request authentication. It changes only that provider and never logs access or refresh tokens.

Install from this checkout and restart PI:

```bash
pi install /absolute/path/to/codex-switch
```

Runtime controls:

```text
/codex-switch-auth-reload status
/codex-switch-auth-reload off
/codex-switch-auth-reload on
```

## Project structure

Core code lives in `src/`: `cli.rs` defines the Clap hierarchy, `main.rs` dispatches commands, `completions.rs` generates and installs dynamic shell completion, `status.rs` and `waybar.rs` render usage, `switch.rs` and `profile.rs` manage profiles, `auto_switch.rs` and `profile_options.rs` manage policy, and `systemd.rs`/`waybar_config.rs` generate integrations.

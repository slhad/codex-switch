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
    [--waybar-hide-minutes-with-days BOOL]
    [--waybar-hide-hours-with-days BOOL]
codex-switch waybar install

codex-switch tracker remove SESSION_ID
codex-switch storage
codex-switch recovery restore

codex-switch completion bash
codex-switch completion install [--force]
```

`STORE` is `codex` or `pi`. The immediate `profile transfer now` operands use exact `STORE/PROFILE` references. On-switch mappings always transfer Codex to PI, so `set`, `enable`, and `disable` take bare profile names. `switch` defaults to `--target both`. `auto set` requires at least one setting and accepts multiple settings atomically. Plain `service logs` prints the journal and exits; `--follow` continues following it.

Examples:

```bash
rtk cargo run -- status --debug all
rtk cargo run -- switch work --target codex
rtk cargo run -- profile save codex work
rtk cargo run -- profile import codex work ./auth.json
rtk cargo run -- profile transfer now codex/work pi/work
rtk cargo run -- profile transfer on-switch disable work
rtk cargo run -- auto set work --priority 100 --pi false
rtk cargo run -- auto run --dry-run
rtk cargo run -- service logs
rtk cargo run -- waybar print
rtk cargo run -- storage
```

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

`waybar print` emits JSON with the active profile in `text` and saved profile details in `tooltip` and `alt`. `waybar install` adds or updates `custom/codex-usage` while preserving unrelated settings and writing a timestamped backup before editing. Generated configuration invokes `codex-switch waybar print`.

Format tokens include `{usage_block}`, `{usage_block_pango}`, `{icon}`, `{time_icon}`, `{5h_pct}`, `{7d_pct}`, `{monthly_used_pct}`, `{status}`, `{profile}`, `{provider}`, `{email}`, `{pct}`, `{reset}`, and `{win}`.

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

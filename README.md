# codex-switch

Rust CLI for switching between Codex auth profiles and inspecting OAuth-backed usage for both the live Codex auth and PI agent auth.

## Project Structure
Core code lives in `src/`:

- `main.rs` wires CLI parsing to runtime actions.
- `cli.rs` defines flags and arguments with `clap`.
- `data.rs` holds shared data structures for auth files, decoded JWT claims, usage payloads, and filesystem context.
- `status.rs` prints live auth state, PI agent auth state, and usage summaries.
- `rate_limit.rs` fetches usage from the OAuth-backed ChatGPT usage endpoint and handles token refresh. OAuth refresh client IDs are derived from existing JWT claims rather than hardcoded.
- `jwt.rs` decodes JWT payloads for email, audience, and PI/OpenAI claim inspection.
- `waybar.rs` prints Waybar JSON for active usage text plus all-profile tooltip/alt data.
- `waybar_config.rs` installs or updates the user Waybar module definition/layout without replacing unrelated config.
- `switch.rs`, `profile.rs`, `process.rs`, and `install.rs` each own one functional area.

Build artifacts go to `target/`. Temporary planning notes live in `tmp/` and should not be treated as source of truth.

## Usage
This project is OAuth-only. It reads `~/.codex/auth.json` and `~/.pi/agent/auth.json`, refreshes tokens when needed, derives the OAuth refresh client ID from JWT claims, and queries the ChatGPT usage endpoint for personal 5-hour/7-day windows or Business monthly credit spend controls.

Before a profile switch, the current live auth is now stored as a single recovery snapshot inside `~/.local/state/codex-switch/accounts.json`. Use `--restore-last` to put that snapshot back into `~/.codex/auth.json`.

Saved Codex and PI profiles are managed under `~/.local/state/codex-switch/profiles/` rather than alongside the live auth files. Status and Waybar tooltip/alt output can refresh and display quota/reset data for saved profiles, not just the active live auth.

A `codex` → `pi` transfer such as `--transfer-profile codex/mate pi/mate` is remembered in `~/.local/state/codex-switch/profile-transfers.json`. A later normal switch to that Codex profile regenerates the mapped PI profile from the latest saved Codex OAuth tokens and activates it. Profiles without a remembered transfer are unchanged, so a PI-native profile such as `me` remains independent. The default status command marks remembered mappings as `[transfer→pi:PROFILE]`. Profile names may contain only ASCII letters, digits, `_`, and `-`.

## Build And Test
Use the local `rtk` wrapper for shell commands in this repo.

- `rtk cargo build` builds the debug binary.
- `rtk cargo build --release` builds the optimized binary at `target/release/codex-switch`.
- `rtk cargo test` runs unit tests.
- `rtk scripts/check-coverage.sh` runs unit tests with LLVM coverage and requires region, function, line, and branch coverage to each be at least 95%.
- `rtk cargo fmt` formats the crate with `rustfmt`.
- `rtk cargo run -- --help` checks CLI behavior quickly.
- `rtk cargo run -- --debug-usage` prints the raw live Codex OAuth usage payload.
- `rtk cargo run -- --debug-pi-usage` prints the raw PI agent OAuth usage payload.
- `rtk cargo run -- --waybar` prints Waybar JSON for Codex usage: active profile in `text`, all saved Codex/PI profiles in `tooltip` and `alt`.
- `rtk cargo run -- --install-waybar-config` installs or updates the `custom/codex-usage` Waybar module while preserving existing user config and writing timestamped backups. If `~/.config/waybar/common.jsonc` exists it is used for the shared module definition; otherwise the module is added inline to the standard Waybar `config.jsonc` object(s).
- `rtk cargo run -- --storage` shows where live auth, saved profiles, and tracker files are stored.
- `rtk cargo run -- --stop-remote` safely stops the Codex app server that owns `~/.codex/app-server-control/app-server-control.sock`, including SSH-bootstrapped servers that `codex remote-control stop` cannot manage.
- `rtk cargo run -- --save-profile codex me` saves the current live Codex auth into the managed `me` profile.
- `rtk cargo run -- --save-profile pi me` saves the current live PI auth into the managed `me` profile.
- `rtk cargo run -- --import-profile work ~/Downloads/auth.json` validates and imports a Codex OAuth `auth.json` into the managed `work` profile. It displays the imported auth information but does not modify live auth or activate the profile.

Examples:

```bash
rtk target/release/codex-switch --install-link
rtk target/release/codex-switch --import-profile work ~/Downloads/auth.json
rtk target/release/codex-switch --force --import-profile work ~/Downloads/auth.json
```

## CLI Flags
The current CLI surface is:

- `-k`, `--kill` to stop running Codex desktop instances and the remote app server before switching
- `--stop-remote` to stop only the Codex app server that owns the remote-control socket; the process command line is validated before it is signaled
- `--debug-usage` to print the raw live Codex OAuth usage payload
- `--debug-pi-usage` to print the raw PI agent OAuth usage payload
- `--waybar` to print Waybar JSON for Codex usage; `text` is active-profile only, while default `tooltip` and `alt` include all saved Codex/PI profiles
- `--waybar-hide-minutes-with-days <true|false>` to omit minutes from Waybar reset durations when days are present; enabled by default to keep the module compact
- `--waybar-hide-hours-with-days <true|false>` to omit hours from Waybar reset durations when days are present; also enabled by default, so day-based resets normally render as just `6d`
- `--format` to customize the Waybar text output; supports `{usage_block}`, `{usage_block_pango}`, `{icon}`, `{icon_plain}`, `{time_icon}`, `{time_icon_plain}`, `{5h_block}`, `{5h_block_pango}`, `{5h_pct}`, `{7d_pct}`, `{5h_reset}`, `{7d_reset}`, `{monthly_limit}`, `{monthly_used}`, `{monthly_remaining}`, `{monthly_used_pct}`, `{monthly_remaining_pct}`, `{monthly_reset}`, `{status}`, `{profile}`, `{provider}`, `{email}`, `{pct}`, `{reset}`, and `{win}`. The usage block automatically renders personal rate limits or Business monthly credits; the 5-hour block tokens expand to nothing when that quota window is absent.
- `--tooltip-format` to customize the Waybar tooltip for the active profile; omit it to show all saved Codex and PI profiles
- `--install-waybar-config` to add/update `custom/codex-usage` in Waybar config without replacing unrelated settings; it uses `~/.config/waybar/common.jsonc` when present, otherwise it edits standard inline `~/.config/waybar/config.jsonc` object(s)
- `--codex` with `[PROFILE]` to switch only the Codex auth profile
- `--pi` with `[PROFILE]` to switch only the PI auth profile
- `--force` to switch even if the current auth does not match a known profile
- `--install-link` to install the `codex-switch` symlink
- `--remove-link` to remove the `codex-switch` symlink
- `--restore-last` to restore the last tracker-backed auth snapshot
- `--storage` to show where live auth, saved profiles, and tracker files are stored
- `--save-profile STORE NAME` to save the current live `codex` or `pi` auth into a named profile
- `--transfer-profile SOURCE_STORE/SOURCE_PROFILE TARGET_STORE/TARGET_PROFILE` (for example, `--transfer-profile codex/mate pi/mate`) to transfer OAuth auth immediately; Codex-to-PI mappings are remembered for future normal switches
- `--import-profile NAME AUTH_JSON` to import a Codex OAuth file into `~/.local/state/codex-switch/profiles/codex/auth.json.NAME`; after importing it displays the profile path, email, account, auth mode, and last refresh without changing live `~/.codex/auth.json`
- `--force --import-profile NAME AUTH_JSON` to replace an existing imported profile; without `--force`, existing profiles are preserved
- `[PROFILE]` to switch to a named saved profile

## Development Notes
Follow standard Rust style and keep formatting `rustfmt`-clean. Use 4-space indentation, `snake_case` for functions/modules, `PascalCase` for structs/enums, and short focused modules. Prefer explicit error messages through the existing `die(...)` helper for user-facing failures. Keep CLI text stable and concise because this tool is intended for terminal use.

No usable Git history is present in this working copy, so follow a simple imperative style for commits, for example: `Add install/remove link flags`. Keep commits scoped to one behavioral change.

PRs should include:

- a short summary of the user-visible change
- commands used for verification
- any filesystem side effects, such as refreshing `~/.codex/auth.json` tokens or creating/removing symlinks in `~/bin`

# Agent Guidance

Use the local `rtk` wrapper for shell commands in this repo.

Prefer minimal patches. Do not commit `target/`, and do not overwrite existing user binaries or links during install flows unless the task explicitly requires it.

When installing the command for regular use, prefer invoking the release binary directly so the symlink points at `target/release/codex-switch` rather than a debug build.

This project is OAuth-only. Prefer reading `~/.codex/auth.json` and `~/.pi/agent/auth.json` plus the usage endpoint over local session log heuristics. Do not hardcode OAuth client IDs; derive them from available JWT claims such as `access_token.client_id`, `id_token.aud`, or PI `access.client_id`.

Keep source, docs, fixtures, and committed examples free of personal PII, account IDs, session IDs, and real tokens. Runtime files under `~/.codex`, `~/.pi`, and `~/.local/state/codex-switch` may contain user data and should not be treated as source artifacts.

Add unit tests close to the code they verify using `#[cfg(test)]` modules. Prefer deterministic filesystem tests that create and clean up temporary directories. Region, function, line, and branch coverage must each stay at or above 95%; verify with `rtk scripts/check-coverage.sh` after meaningful changes. After the final code change, always build the release binary with `rtk cargo build --release`. When changing CLI behavior, verify both `rtk cargo test` and a targeted manual run such as `rtk cargo run -- status --debug codex`, `rtk cargo run -- waybar print`, `rtk cargo run -- waybar install`, or `rtk target/release/codex-switch link uninstall`. Waybar config installation must preserve unrelated user settings and write backups before editing files under `~/.config/waybar`.

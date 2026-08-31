use crate::data::die;
use std::ffi::OsStr;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const PLUGINS_DIR: &str = ".config/omarchy/plugins";
const BACKUPS_DIR: &str = ".config/omarchy/plugin-backups";
const IGNORED_LOCAL_ENTRIES: [&str; 4] = [".git", "node_modules", "target", "tmp"];
const ALLOWED_GIT_SCHEMES: [&str; 9] = [
    "file", "ftp", "ftps", "git", "git+ssh", "http", "https", "ssh", "ssh+git",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Install,
    Update,
}

enum Source<'a> {
    Git(&'a str),
    Local(&'a Path),
}

pub fn install(
    positional: Option<&str>,
    git: Option<&str>,
    local: Option<&Path>,
    enable: bool,
    assume_yes: bool,
) {
    run(
        Operation::Install,
        positional,
        git,
        local,
        enable,
        assume_yes,
    )
    .unwrap_or_else(|error| die(&format!("Omarchy plugin install failed: {}", error)));
}

pub fn update(positional: Option<&str>, git: Option<&str>, local: Option<&Path>, assume_yes: bool) {
    run(Operation::Update, positional, git, local, false, assume_yes)
        .unwrap_or_else(|error| die(&format!("Omarchy plugin update failed: {}", error)));
}

fn run(
    operation: Operation,
    positional: Option<&str>,
    git: Option<&str>,
    local: Option<&Path>,
    enable: bool,
    assume_yes: bool,
) -> Result<(), String> {
    let source = source_from_args(positional, git, local)?;
    let home = home_dir()?;
    let plugins_dir = home.join(PLUGINS_DIR);
    std::fs::create_dir_all(&plugins_dir).map_err(|error| {
        format!(
            "failed to create plugin directory {}: {}",
            plugins_dir.display(),
            error
        )
    })?;

    let stage = create_stage_dir(&plugins_dir)?;
    let result = stage_source(source, &stage)
        .and_then(|_| inspect_plugin_folder(&stage))
        .and_then(|id| {
            validate_with_omarchy(&stage)?;
            Ok(id)
        });
    let id = match result {
        Ok(id) => id,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(error);
        }
    };

    let target = plugins_dir.join(&id);
    let target_exists = std::fs::symlink_metadata(&target).is_ok();
    match operation {
        Operation::Install if target_exists => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(format!(
                "plugin `{}` is already installed; update it with `codex-switch omarchy update`",
                id
            ));
        }
        Operation::Update if !target_exists => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(format!(
                "plugin `{}` is not installed; install it with `codex-switch omarchy install`",
                id
            ));
        }
        _ => {}
    }
    if target_exists && (!target.is_dir() || target.is_symlink()) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(format!(
            "plugin target {} must be a real directory",
            target.display()
        ));
    }

    let verb = match operation {
        Operation::Install => "install",
        Operation::Update => "update",
    };
    if let Err(error) = confirm(assume_yes, verb, &id) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(error);
    }

    let backup = if operation == Operation::Update {
        match move_to_backup(&home, &target, &id) {
            Ok(backup) => Some(backup),
            Err(error) => {
                let _ = std::fs::remove_dir_all(&stage);
                return Err(error);
            }
        }
    } else {
        None
    };

    if let Err(error) = std::fs::rename(&stage, &target) {
        if let Some(backup) = backup.as_ref() {
            let _ = std::fs::rename(backup, &target);
        }
        let _ = std::fs::remove_dir_all(&stage);
        return Err(format!(
            "failed to install plugin into {}: {}",
            target.display(),
            error
        ));
    }

    reload_shell()?;
    if enable {
        wait_for_plugin(&id)?;
        enable_plugin(&id)?;
    }

    match (operation, backup) {
        (Operation::Install, _) => {
            println!("Installed Omarchy plugin `{}` at {}", id, target.display())
        }
        (Operation::Update, Some(backup)) => println!(
            "Updated Omarchy plugin `{}` at {}\nBackup: {}",
            id,
            target.display(),
            backup.display()
        ),
        (Operation::Update, None) => println!("Updated Omarchy plugin `{}`", id),
    }
    Ok(())
}

fn source_from_args<'a>(
    positional: Option<&'a str>,
    git: Option<&'a str>,
    local: Option<&'a Path>,
) -> Result<Source<'a>, String> {
    match (positional, git, local) {
        (Some(source), _, _) => {
            if git.is_some() || local.is_some() {
                return Err("use only one of SOURCE, --git, or --local".to_string());
            }
            if Path::new(source).is_dir() {
                Ok(Source::Local(Path::new(source)))
            } else {
                validate_git_url(source)?;
                Ok(Source::Git(source))
            }
        }
        (None, Some(url), None) if !url.trim().is_empty() => {
            validate_git_url(url)?;
            Ok(Source::Git(url))
        }
        (None, None, Some(path)) => Ok(Source::Local(path)),
        (None, Some(_), None) => Err("--git requires a repository URL".to_string()),
        (None, None, None) => {
            Err("specify exactly one of SOURCE, --git URL, or --local [DIRECTORY]".to_string())
        }
        (None, Some(_), Some(_)) => Err("use only one of --git or --local".to_string()),
    }
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())
}

fn create_stage_dir(plugins_dir: &Path) -> Result<PathBuf, String> {
    let nonce = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    for attempt in 0..100u32 {
        let stage = plugins_dir.join(format!(
            ".codex-switch-plugin-stage-{}-{}-{}",
            std::process::id(),
            nonce,
            attempt
        ));
        match std::fs::create_dir(&stage) {
            Ok(()) => return Ok(stage),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create staging directory {}: {}",
                    stage.display(),
                    error
                ))
            }
        }
    }
    Err("could not allocate a unique plugin staging directory".to_string())
}

fn stage_source(source: Source<'_>, stage: &Path) -> Result<(), String> {
    match source {
        Source::Git(url) => clone_git(url, stage),
        Source::Local(path) => {
            let metadata = std::fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "cannot inspect local plugin directory {}: {}",
                    path.display(),
                    error
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "local plugin source {} must be a real directory",
                    path.display()
                ));
            }
            copy_directory(path, stage)
        }
    }
}

fn validate_git_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("git URL cannot be empty".to_string());
    }
    if url.starts_with('-') || url.contains("::") {
        return Err(format!("refusing unsafe git URL `{}`", url));
    }
    if let Some((scheme, _)) = url.split_once("://") {
        if !ALLOWED_GIT_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
            return Err(format!("unsupported git URL scheme `{}`", scheme));
        }
    }
    Ok(())
}

fn clone_git(url: &str, stage: &Path) -> Result<(), String> {
    let status = Command::new("omarchy-git-url-check")
        .arg(url)
        .status()
        .map_err(|error| format!("failed to run `omarchy-git-url-check`: {}", error))?;
    if !status.success() {
        return Err(format!("Omarchy rejected git URL `{}`", url));
    }

    // git clone expects to create the destination itself. The directory is
    // reserved before cloning so a second installer cannot choose the same path.
    std::fs::remove_dir(stage).map_err(|error| {
        format!(
            "failed to prepare plugin staging directory {}: {}",
            stage.display(),
            error
        )
    })?;
    let mut command = Command::new("git");
    command
        .args(["clone", "--", url])
        .arg(stage)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env(
            "GIT_SSH_COMMAND",
            std::env::var("GIT_SSH_COMMAND").unwrap_or_else(|_| "ssh -oBatchMode=yes".to_string()),
        );
    let output = command
        .output()
        .map_err(|error| format!("failed to start git: {}", error))?;
    if !output.status.success() {
        let detail = command_output_detail(&output);
        return Err(if detail.is_empty() {
            format!("git clone failed for `{}`", url)
        } else {
            format!("git clone failed for `{}`: {}", url, detail)
        });
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destination).map_err(|error| {
        format!(
            "failed to create plugin staging directory {}: {}",
            destination.display(),
            error
        )
    })?;
    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("failed to read local plugin directory: {}", error))?
    {
        let entry = entry.map_err(|error| format!("failed to read plugin entry: {}", error))?;
        if ignored_local_entry(&entry.file_name()) {
            continue;
        }
        let target = destination.join(entry.file_name());
        copy_entry(&entry.path(), &target)?;
    }
    Ok(())
}

fn copy_entry(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect {}: {}", source.display(), error))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "plugin source contains forbidden symlink {}",
            source.display()
        ));
    }
    if metadata.is_dir() {
        std::fs::create_dir(destination)
            .map_err(|error| format!("failed to create {}: {}", destination.display(), error))?;
        for entry in std::fs::read_dir(source)
            .map_err(|error| format!("failed to read {}: {}", source.display(), error))?
        {
            let entry = entry.map_err(|error| format!("failed to read plugin entry: {}", error))?;
            if ignored_local_entry(&entry.file_name()) {
                continue;
            }
            copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
        }
        std::fs::set_permissions(destination, metadata.permissions()).map_err(|error| {
            format!(
                "failed to preserve permissions on {}: {}",
                destination.display(),
                error
            )
        })?;
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(format!(
            "plugin source entry {} is not a regular file",
            source.display()
        ));
    }
    std::fs::copy(source, destination)
        .map_err(|error| format!("failed to copy {}: {}", source.display(), error))?;
    std::fs::set_permissions(destination, metadata.permissions()).map_err(|error| {
        format!(
            "failed to preserve permissions on {}: {}",
            destination.display(),
            error
        )
    })?;
    Ok(())
}

fn ignored_local_entry(name: &OsStr) -> bool {
    IGNORED_LOCAL_ENTRIES
        .iter()
        .any(|ignored| name == OsStr::new(ignored))
}

fn inspect_plugin_folder(folder: &Path) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(folder).map_err(|error| {
        format!(
            "cannot inspect plugin folder {}: {}",
            folder.display(),
            error
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "plugin folder {} must be a real directory",
            folder.display()
        ));
    }
    reject_symlinks(folder)?;
    plugin_id_from_manifest(folder)
}

fn plugin_id_from_manifest(folder: &Path) -> Result<String, String> {
    let path = folder.join("manifest.json");
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {}", path.display(), error))?;
    let manifest: serde_json::Value = serde_json::from_str(&content)
        .map_err(|error| format!("invalid plugin manifest {}: {}", path.display(), error))?;
    let id = manifest
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "plugin manifest must contain a string `id`".to_string())?;
    validate_plugin_id(id)?;
    Ok(id.to_string())
}

fn validate_plugin_id(id: &str) -> Result<(), String> {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return Err("plugin manifest `id` cannot be empty".to_string());
    };
    if !first.is_ascii_alphanumeric()
        || !chars.all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        || id.contains("..")
    {
        return Err(format!(
            "plugin manifest id `{}` must match [A-Za-z0-9][A-Za-z0-9._-]* and cannot contain `..`",
            id
        ));
    }
    Ok(())
}

fn reject_symlinks(folder: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(folder)
        .map_err(|error| format!("failed to inspect plugin folder: {}", error))?
    {
        let entry = entry.map_err(|error| format!("failed to inspect plugin entry: {}", error))?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {}", path.display(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "plugin folder contains forbidden symlink {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            reject_symlinks(&path)?;
        }
    }
    Ok(())
}

fn validate_with_omarchy(folder: &Path) -> Result<(), String> {
    let output = Command::new("omarchy")
        .args(["plugin", "validate"])
        .arg(folder)
        .output()
        .map_err(|error| format!("failed to run `omarchy plugin validate`: {}", error))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = command_output_detail(&output);
    if detail.is_empty() {
        Err("omarchy plugin validation failed".to_string())
    } else {
        Err(format!("omarchy plugin validation failed: {}", detail))
    }
}

fn move_to_backup(home: &Path, target: &Path, id: &str) -> Result<PathBuf, String> {
    let backup_dir = home.join(BACKUPS_DIR);
    std::fs::create_dir_all(&backup_dir).map_err(|error| {
        format!(
            "failed to create plugin backup directory {}: {}",
            backup_dir.display(),
            error
        )
    })?;
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    for attempt in 0..100u32 {
        let name = if attempt == 0 {
            format!("{}-{}", id, stamp)
        } else {
            format!("{}-{}-{}", id, stamp, attempt)
        };
        let backup = backup_dir.join(name);
        if std::fs::symlink_metadata(&backup).is_ok() {
            continue;
        }
        std::fs::rename(target, &backup).map_err(|error| {
            format!(
                "failed to back up {} to {}: {}",
                target.display(),
                backup.display(),
                error
            )
        })?;
        return Ok(backup);
    }
    Err("could not allocate a unique plugin backup path".to_string())
}

fn confirm(assume_yes: bool, verb: &str, id: &str) -> Result<(), String> {
    if assume_yes {
        return Ok(());
    }
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return Err(format!(
            "refusing to {} unsandboxed plugin `{}` without confirmation; pass --yes",
            verb, id
        ));
    }
    eprint!(
        "{} unsandboxed plugin `{}`? [y/N] ",
        if verb == "install" {
            "Install"
        } else {
            "Update"
        },
        id
    );
    io::stderr()
        .flush()
        .map_err(|error| format!("failed to prompt for confirmation: {}", error))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("failed to read confirmation: {}", error))?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err("aborted".to_string())
    }
}

fn reload_shell() -> Result<(), String> {
    let rescan = Command::new("omarchy-shell")
        .args(["shell", "rescanPlugins"])
        .output();
    if rescan.as_ref().is_ok_and(|output| output.status.success()) {
        return Ok(());
    }

    let restart = Command::new("omarchy")
        .args(["restart", "shell"])
        .output()
        .map_err(|error| format!("failed to restart Omarchy shell: {}", error))?;
    if restart.status.success() {
        return Ok(());
    }

    let detail = command_output_detail(&restart);
    if detail.is_empty() {
        Err("plugin installed, but Omarchy shell reload failed".to_string())
    } else {
        Err(format!(
            "plugin installed, but Omarchy shell reload failed: {}",
            detail
        ))
    }
}

fn wait_for_plugin(id: &str) -> Result<(), String> {
    let mut last_detail = String::new();
    for _ in 0..40 {
        let output = Command::new("omarchy-plugin-list")
            .arg("--json")
            .output()
            .map_err(|error| format!("failed to list Omarchy plugins: {}", error))?;
        if output.status.success() {
            let known = serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .ok()
                .and_then(|plugins| plugins.as_array().cloned())
                .is_some_and(|plugins| {
                    plugins.iter().any(|plugin| {
                        plugin.get("id").and_then(serde_json::Value::as_str) == Some(id)
                    })
                });
            if known {
                return Ok(());
            }
        } else {
            last_detail = command_output_detail(&output);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if last_detail.is_empty() {
        Err(format!("plugin `{}` is not known to the Omarchy shell", id))
    } else {
        Err(format!(
            "plugin `{}` is not known to the Omarchy shell: {}",
            id, last_detail
        ))
    }
}

fn enable_plugin(id: &str) -> Result<(), String> {
    let output = Command::new("omarchy")
        .args(["plugin", "enable", id])
        .output()
        .map_err(|error| format!("failed to enable plugin `{}`: {}", id, error))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = command_output_detail(&output);
    if detail.is_empty() {
        Err(format!(
            "plugin `{}` was installed but could not be enabled",
            id
        ))
    } else {
        Err(format!(
            "plugin `{}` was installed but could not be enabled: {}",
            id, detail
        ))
    }
}

fn command_output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        copy_directory, inspect_plugin_folder, source_from_args, validate_git_url, Source,
    };
    use std::os::unix::fs::symlink;
    use std::path::Path;

    fn fixture(name: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "codex-switch-omarchy-plugin-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn validates_git_urls_without_running_git() {
        assert!(validate_git_url("https://github.com/example/plugin.git").is_ok());
        assert!(validate_git_url("git@github.com:example/plugin.git").is_ok());
        assert!(validate_git_url("file:///tmp/plugin").is_ok());
        assert!(validate_git_url("--upload-pack=sh").is_err());
        assert!(validate_git_url("ext::sh -c whoami").is_err());
        assert!(validate_git_url("gopher://example/plugin").is_err());
    }

    #[test]
    fn requires_one_plugin_source() {
        assert!(source_from_args(None, None, None).is_err());
        assert!(source_from_args(
            None,
            Some("https://example.com/plugin.git"),
            Some(Path::new("."))
        )
        .is_err());
        assert!(matches!(
            source_from_args(None, Some("https://example.com/plugin.git"), None).unwrap(),
            Source::Git(_)
        ));
        assert!(matches!(
            source_from_args(None, None, Some(Path::new("."))).unwrap(),
            Source::Local(_)
        ));
        assert!(matches!(
            source_from_args(Some("https://example.com/plugin.git"), None, None).unwrap(),
            Source::Git(_)
        ));
        assert!(matches!(
            source_from_args(Some("."), None, None).unwrap(),
            Source::Local(_)
        ));
    }

    #[test]
    fn copies_local_plugin_without_vcs_or_build_output() {
        let base = fixture("copy");
        let source = base.join("source");
        let destination = base.join("destination");
        std::fs::create_dir_all(source.join("omarchy")).unwrap();
        std::fs::create_dir_all(source.join(".git")).unwrap();
        std::fs::create_dir_all(source.join("target")).unwrap();
        std::fs::write(
            source.join("manifest.json"),
            r#"{"schemaVersion":1,"id":"io.github.example.plugin"}"#,
        )
        .unwrap();
        std::fs::write(source.join("omarchy/BarWidget.qml"), "Item {}\n").unwrap();
        std::fs::write(source.join(".git/HEAD"), "ref\n").unwrap();
        std::fs::write(source.join("target/debug"), "build\n").unwrap();

        copy_directory(&source, &destination).unwrap();
        assert!(destination.join("manifest.json").exists());
        assert!(destination.join("omarchy/BarWidget.qml").exists());
        assert!(!destination.join(".git").exists());
        assert!(!destination.join("target").exists());
        assert_eq!(
            inspect_plugin_folder(&destination).unwrap(),
            "io.github.example.plugin"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_invalid_manifest_ids_and_symlinks() {
        let base = fixture("reject");
        let invalid = base.join("invalid");
        std::fs::create_dir_all(&invalid).unwrap();
        std::fs::write(invalid.join("manifest.json"), r#"{"id":"../escape"}"#).unwrap();
        assert!(inspect_plugin_folder(&invalid).is_err());

        let linked = base.join("linked");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked.join("manifest.json"), r#"{"id":"valid.plugin"}"#).unwrap();
        symlink("manifest.json", linked.join("manifest-link.json")).unwrap();
        assert!(inspect_plugin_folder(&linked).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}

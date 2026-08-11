use crate::data::{AuthFile, Context};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub struct ShadowHomeResult {
    pub shared_home: PathBuf,
    pub shadow_home: PathBuf,
    pub auth_path: PathBuf,
    pub profile_path: PathBuf,
    pub changed: bool,
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn resolve_shadow_home(
    ctx: &Context,
    name: &str,
    requested: Option<&Path>,
) -> Result<PathBuf, String> {
    let shared_home = ctx
        .live_auth
        .parent()
        .ok_or_else(|| "live Codex auth path has no parent directory".to_string())?;
    let home = shared_home
        .parent()
        .ok_or_else(|| "cannot determine the home directory from live Codex auth".to_string())?;
    let requested = requested
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".codex-t3").join(name));
    let expanded = if requested == Path::new("~") {
        home.to_path_buf()
    } else if let Ok(relative) = requested.strip_prefix("~/") {
        home.join(relative)
    } else if requested
        .components()
        .next()
        .is_some_and(|component| component.as_os_str().to_string_lossy().starts_with('~'))
    {
        return Err("shadow home supports `~` and `~/...`, but not `~USER` paths".to_string());
    } else {
        requested
    };
    normalize_absolute_path(&expanded)
}

pub fn bootstrap_shadow_home(
    ctx: &Context,
    name: &str,
    requested: Option<&Path>,
    force: bool,
) -> Result<ShadowHomeResult, String> {
    crate::switch::validate_profile_name(name)?;
    let shared_home = normalize_absolute_path(
        ctx.live_auth
            .parent()
            .ok_or_else(|| "live Codex auth path has no parent directory".to_string())?,
    )?;
    let shadow_home = resolve_shadow_home(ctx, name, requested)?;
    if shadow_home == shared_home {
        return Err(format!(
            "shadow home must differ from the shared Codex home: {}",
            shared_home.display()
        ));
    }

    let source = ctx.profile_path(name);
    let metadata = fs::metadata(&source).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("saved Codex profile `{name}` does not exist")
        } else {
            format!(
                "failed to inspect saved Codex profile {}: {error}",
                source.display()
            )
        }
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "saved Codex profile is not a regular file: {}",
            source.display()
        ));
    }
    let content = fs::read_to_string(&source).map_err(|error| {
        format!(
            "failed to read saved Codex profile {}: {error}",
            source.display()
        )
    })?;
    serde_json::from_str::<AuthFile>(&content)
        .map_err(|error| format!("invalid saved Codex profile {}: {error}", source.display()))?;

    fs::create_dir_all(&shadow_home).map_err(|error| {
        format!(
            "failed to create T3 Code shadow home {}: {error}",
            shadow_home.display()
        )
    })?;
    let auth_path = shadow_home.join("auth.json");
    let existing = match fs::symlink_metadata(&auth_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "refusing to replace symlinked shadow auth: {}",
                auth_path.display()
            ));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "shadow auth path is not a regular file: {}",
                auth_path.display()
            ));
        }
        Ok(_) => Some(fs::read_to_string(&auth_path).map_err(|error| {
            format!(
                "failed to read shadow auth {}: {error}",
                auth_path.display()
            )
        })?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "failed to inspect shadow auth {}: {error}",
                auth_path.display()
            ));
        }
    };
    let auth_changed = existing.as_deref() != Some(content.as_str());
    if auth_changed && existing.is_some() && !force {
        return Err(format!(
            "shadow auth already exists at {}; use --force to replace it",
            auth_path.display()
        ));
    }

    if auth_changed {
        let temporary = shadow_home.join(format!(".auth.json.{}.tmp", std::process::id()));
        fs::write(&temporary, &content)
            .map_err(|error| format!("failed to write temporary shadow auth: {error}"))?;
        if let Err(error) = fs::set_permissions(&temporary, metadata.permissions()) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("failed to secure temporary shadow auth: {error}"));
        }
        if let Err(error) = fs::rename(&temporary, &auth_path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "failed to install shadow auth {}: {error}",
                auth_path.display()
            ));
        }
    }

    let source_metadata = fs::symlink_metadata(&source).map_err(|error| {
        format!(
            "failed to inspect saved Codex profile {}: {error}",
            source.display()
        )
    })?;
    let profile_linked = if source_metadata.file_type().is_symlink() {
        let target = fs::read_link(&source).map_err(|error| {
            format!(
                "failed to read saved Codex profile link {}: {error}",
                source.display()
            )
        })?;
        let resolved = normalize_absolute_path(
            &source
                .parent()
                .ok_or_else(|| "saved profile path has no parent directory".to_string())?
                .join(target),
        )?;
        if resolved != auth_path {
            return Err(format!(
                "saved Codex profile is already linked elsewhere: {}",
                source.display()
            ));
        }
        true
    } else {
        false
    };

    if !profile_linked {
        let temporary_link = source.with_file_name(format!(
            ".{}.{}.tmp",
            source
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("auth"),
            std::process::id()
        ));
        symlink(&auth_path, &temporary_link)
            .map_err(|error| format!("failed to create saved profile link: {error}"))?;
        if let Err(error) = fs::rename(&temporary_link, &source) {
            let _ = fs::remove_file(&temporary_link);
            return Err(format!(
                "failed to link saved profile {} to shadow auth: {error}",
                source.display()
            ));
        }
    }

    Ok(ShadowHomeResult {
        shared_home,
        shadow_home,
        auth_path,
        profile_path: source,
        changed: auth_changed || !profile_linked,
    })
}

#[cfg(test)]
mod tests {
    use super::bootstrap_shadow_home;
    use crate::data::Context;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::PathBuf;

    fn context(name: &str) -> (Context, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "codex-switch-t3-shadow-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let state_dir = base.join("state");
        let ctx = Context {
            live_auth: base.join("home/.codex/auth.json"),
            pi_auth: base.join("pi.json"),
            tracker_file: state_dir.join("accounts.json"),
            state_dir,
        };
        std::fs::create_dir_all(ctx.live_auth.parent().unwrap()).unwrap();
        std::fs::create_dir_all(ctx.codex_profiles_dir()).unwrap();
        (ctx, base)
    }

    fn auth(account: Option<&str>, token: &str) -> String {
        serde_json::json!({"tokens": {"id_token": token, "account_id": account}}).to_string()
    }

    #[test]
    fn bootstraps_without_touching_live_auth() {
        let (ctx, base) = context("create");
        let personal = auth(Some("acct-personal"), "personal");
        let work = auth(Some("acct-work"), "work");
        std::fs::write(&ctx.live_auth, &personal).unwrap();
        std::fs::write(ctx.profile_path("work"), &work).unwrap();
        let mut permissions = std::fs::metadata(ctx.profile_path("work"))
            .unwrap()
            .permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(ctx.profile_path("work"), permissions).unwrap();

        let result = bootstrap_shadow_home(&ctx, "work", None, false).unwrap();
        assert!(result.changed);
        assert_eq!(result.shared_home, base.join("home/.codex"));
        assert_eq!(result.shadow_home, base.join("home/.codex-t3/work"));
        assert_eq!(std::fs::read_to_string(&result.auth_path).unwrap(), work);
        assert_eq!(result.profile_path, ctx.profile_path("work"));
        assert!(std::fs::symlink_metadata(&result.profile_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read_link(&result.profile_path).unwrap(),
            result.auth_path
        );
        assert_eq!(std::fs::read_to_string(&ctx.live_auth).unwrap(), personal);
        assert_eq!(
            std::fs::metadata(&result.auth_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let unchanged = bootstrap_shadow_home(&ctx, "work", None, false).unwrap();
        assert!(!unchanged.changed);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn requires_force_for_different_existing_auth() {
        let (ctx, base) = context("force");
        let destination = base.join("custom/../custom/work");
        let work = auth(Some("acct-work"), "work");
        std::fs::write(ctx.profile_path("work"), &work).unwrap();
        std::fs::create_dir_all(base.join("custom/work")).unwrap();
        std::fs::write(base.join("custom/work/auth.json"), auth(None, "old")).unwrap();

        let error = bootstrap_shadow_home(&ctx, "work", Some(&destination), false).unwrap_err();
        assert!(error.contains("use --force"));
        let result = bootstrap_shadow_home(&ctx, "work", Some(&destination), true).unwrap();
        assert_eq!(result.shadow_home, base.join("custom/work"));
        assert_eq!(std::fs::read_to_string(result.auth_path).unwrap(), work);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_unsafe_paths_and_invalid_profiles() {
        let (ctx, base) = context("invalid");
        std::fs::write(ctx.profile_path("work"), auth(None, "work")).unwrap();
        assert!(
            bootstrap_shadow_home(&ctx, "work", Some(ctx.live_auth.parent().unwrap()), false)
                .unwrap_err()
                .contains("must differ")
        );
        assert!(bootstrap_shadow_home(
            &ctx,
            "work",
            Some(PathBuf::from("~other/x").as_path()),
            false
        )
        .unwrap_err()
        .contains("not `~USER`"));
        assert!(bootstrap_shadow_home(&ctx, "missing", None, false)
            .unwrap_err()
            .contains("does not exist"));
        assert!(bootstrap_shadow_home(&ctx, "bad/name", None, false).is_err());

        let linked_target = base.join("linked-auth.json");
        std::fs::write(&linked_target, auth(None, "linked")).unwrap();
        symlink(&linked_target, ctx.profile_path("linked")).unwrap();
        assert!(bootstrap_shadow_home(&ctx, "linked", None, false)
            .unwrap_err()
            .contains("already linked elsewhere"));

        std::fs::write(ctx.profile_path("invalid"), "not json").unwrap();
        assert!(bootstrap_shadow_home(&ctx, "invalid", None, false)
            .unwrap_err()
            .contains("invalid saved Codex profile"));
        std::fs::create_dir(ctx.profile_path("directory")).unwrap();
        assert!(bootstrap_shadow_home(&ctx, "directory", None, false)
            .unwrap_err()
            .contains("not a regular file"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_symlinked_or_non_file_auth() {
        let (ctx, base) = context("auth-path");
        std::fs::write(&ctx.live_auth, auth(None, "personal")).unwrap();
        std::fs::write(ctx.profile_path("work"), auth(None, "work")).unwrap();
        let shadow = base.join("shadow");
        std::fs::create_dir_all(&shadow).unwrap();
        symlink(&ctx.live_auth, shadow.join("auth.json")).unwrap();
        assert!(bootstrap_shadow_home(&ctx, "work", Some(&shadow), true)
            .unwrap_err()
            .contains("symlinked shadow auth"));
        std::fs::remove_file(shadow.join("auth.json")).unwrap();
        std::fs::create_dir(shadow.join("auth.json")).unwrap();
        assert!(bootstrap_shadow_home(&ctx, "work", Some(&shadow), true)
            .unwrap_err()
            .contains("not a regular file"));
        assert_eq!(
            std::fs::read_to_string(&ctx.live_auth).unwrap(),
            auth(None, "personal")
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}

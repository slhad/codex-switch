use std::path::{Path, PathBuf};

use crate::data::die;

const LINK_NAME: &str = "codex-switch";

pub fn install_link() {
    let current_exe = std::env::current_exe()
        .unwrap_or_else(|e| die(&format!("failed to resolve current executable: {}", e)));
    let destinations = link_paths();
    let path_env = std::env::var_os("PATH").unwrap_or_default();

    if let Some(existing) = destinations
        .iter()
        .find(|path| (path.exists() || path.is_symlink()) && path_is_on_path(path, &path_env))
    {
        println!(
            "codex-switch link install skipped: {} already exists",
            existing.display()
        );
        return;
    }

    let destination = select_install_path_with_path(&destinations, &path_env)
        .unwrap_or_else(|| die("neither ~/bin nor ~/.local/bin exists"));
    if destination.exists() || destination.is_symlink() {
        println!(
            "codex-switch link install skipped: {} already exists",
            destination.display()
        );
        return;
    }

    std::os::unix::fs::symlink(&current_exe, &destination).unwrap_or_else(|e| {
        die(&format!(
            "failed to create symlink {} -> {}: {}",
            destination.display(),
            current_exe.display(),
            e
        ))
    });

    println!(
        "installed {} -> {}",
        destination.display(),
        current_exe.display()
    );
}

pub fn remove_link() {
    let mut removed = false;

    for path in link_paths() {
        if path.is_symlink() {
            std::fs::remove_file(&path)
                .unwrap_or_else(|e| die(&format!("failed to remove {}: {}", path.display(), e)));
            println!("removed {}", path.display());
            removed = true;
        }
    }

    if !removed {
        println!("no codex-switch symlink found in ~/bin or ~/.local/bin");
    }
}

fn link_paths() -> Vec<PathBuf> {
    let home = home_dir();
    vec![
        home.join("bin").join(LINK_NAME),
        home.join(".local").join("bin").join(LINK_NAME),
    ]
}

fn select_install_path_with_path(
    destinations: &[PathBuf],
    path_env: &std::ffi::OsStr,
) -> Option<PathBuf> {
    destinations
        .iter()
        .find(|path| path.parent().is_some_and(Path::is_dir) && path_is_on_path(path, path_env))
        .or_else(|| {
            destinations
                .iter()
                .find(|path| path.parent().is_some_and(Path::is_dir))
        })
        .cloned()
}

fn path_is_on_path(path: &Path, path_env: &std::ffi::OsStr) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    std::env::split_paths(path_env).any(|entry| entry == parent)
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set"))
}

#[cfg(test)]
mod tests {
    use super::select_install_path_with_path;

    #[test]
    fn prefers_path_visible_directory() {
        let base = std::env::temp_dir().join(format!("codex-switch-test-{}", std::process::id()));
        let first_dir = base.join("bin");
        let second_dir = base.join(".local").join("bin");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::create_dir_all(&second_dir).unwrap();

        let first = first_dir.join("codex-switch");
        let second = second_dir.join("codex-switch");
        let path_env = std::env::join_paths([second_dir.as_path()]).unwrap();
        assert_eq!(
            select_install_path_with_path(&[first.clone(), second.clone()], &path_env),
            Some(second.clone())
        );

        let empty_path = std::ffi::OsString::new();
        assert_eq!(
            select_install_path_with_path(&[first.clone(), second], &empty_path),
            Some(first)
        );

        std::fs::remove_dir_all(base).unwrap();
    }
}

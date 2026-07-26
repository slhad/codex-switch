use crate::data::Context;
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

fn list_matching_pids(proc_root: &Path, predicate: fn(&[String]) -> bool) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut pids = Vec::new();

    for entry in entries.filter_map(Result::ok) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if read_cmdline(proc_root, pid).is_ok_and(|args| predicate(&args)) {
            pids.push(pid);
        }
    }

    pids.sort_unstable();
    pids.dedup();
    pids
}

fn is_codex_desktop(args: &[String]) -> bool {
    args.first()
        .is_some_and(|executable| executable.contains("/opt/codex-desktop/"))
}

fn list_codex_desktop_pids(proc_root: &Path) -> Vec<u32> {
    list_matching_pids(proc_root, is_codex_desktop)
}

fn list_codex_app_server_pids(proc_root: &Path) -> Vec<u32> {
    list_matching_pids(proc_root, is_codex_app_server)
}

pub fn kill_codex_desktop(_ctx: &Context) {
    let pids = list_codex_desktop_pids(Path::new("/proc"));

    if pids.is_empty() {
        println!("No Codex desktop instances found.");
        return;
    }

    println!("Stopping Codex desktop processes:");
    for pid in pids {
        println!("  pid {}", pid);
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
    }
}

pub fn stop_codex_remote(ctx: &Context) -> Result<(), String> {
    let codex_dir = ctx
        .live_auth
        .parent()
        .ok_or_else(|| "cannot determine the Codex home directory".to_string())?;
    let socket = codex_dir
        .join("app-server-control")
        .join("app-server-control.sock");
    stop_remote_at(&socket, Path::new("/proc"))
}

fn stop_remote_at(socket: &Path, proc_root: &Path) -> Result<(), String> {
    let inode = find_socket_inode(socket, proc_root)?;
    let mut pids = if let Some(inode) = inode {
        let owners = find_socket_owners(inode, proc_root)?;
        if owners.is_empty() {
            return Err(format!(
                "the remote control socket is active, but its owning process could not be identified: {}",
                socket.display()
            ));
        }
        for pid in &owners {
            let args = read_cmdline(proc_root, *pid)?;
            if !is_codex_app_server(&args) {
                return Err(format!(
                    "refusing to stop pid {} because it is not a Codex app-server: {}",
                    pid,
                    args.join(" ")
                ));
            }
        }
        owners
    } else {
        Vec::new()
    };

    pids.extend(list_codex_app_server_pids(proc_root));
    pids.sort_unstable();
    pids.dedup();
    if pids.is_empty() {
        remove_stale_socket(socket)?;
        println!("No Codex remote app server found.");
        return Ok(());
    }

    println!("Stopping Codex remote app server:");
    for pid in &pids {
        println!("  pid {}", pid);
        signal(*pid, Signal::SIGTERM)?;
    }

    if wait_for_remote_to_stop(socket, proc_root, &pids, Duration::from_secs(5))? {
        remove_stale_socket(socket)?;
        println!("Codex remote app server stopped.");
        return Ok(());
    }

    println!("Remote app server did not stop after TERM; sending KILL.");
    for pid in &pids {
        signal(*pid, Signal::SIGKILL)?;
    }

    if !wait_for_remote_to_stop(socket, proc_root, &pids, Duration::from_secs(2))? {
        return Err("Codex remote app server is still running".to_string());
    }

    remove_stale_socket(socket)?;
    println!("Codex remote app server stopped.");
    Ok(())
}

fn find_socket_inode(socket: &Path, proc_root: &Path) -> Result<Option<u64>, String> {
    let contents = std::fs::read_to_string(proc_root.join("net/unix"))
        .map_err(|e| format!("cannot read /proc/net/unix: {e}"))?;
    let wanted = socket.to_string_lossy();

    for line in contents.lines().skip(1) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() >= 8 && fields[7] == wanted {
            let inode = fields[6]
                .parse::<u64>()
                .map_err(|e| format!("invalid socket inode in /proc/net/unix: {e}"))?;
            return Ok(Some(inode));
        }
    }
    Ok(None)
}

fn find_socket_owners(inode: u64, proc_root: &Path) -> Result<Vec<u32>, String> {
    let target = format!("socket:[{inode}]");
    let entries = std::fs::read_dir(proc_root)
        .map_err(|e| format!("cannot inspect {}: {e}", proc_root.display()))?;
    let mut owners = Vec::new();

    for entry in entries.filter_map(Result::ok) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        if fds.filter_map(Result::ok).any(|fd| {
            std::fs::read_link(fd.path()).is_ok_and(|link| link.to_string_lossy() == target)
        }) {
            owners.push(pid);
        }
    }

    owners.sort_unstable();
    owners.dedup();
    Ok(owners)
}

fn read_cmdline(proc_root: &Path, pid: u32) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(proc_root.join(pid.to_string()).join("cmdline"))
        .map_err(|e| format!("cannot inspect command line for pid {pid}: {e}"))?;
    Ok(bytes
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect())
}

fn is_codex_app_server(args: &[String]) -> bool {
    let executable_is_codex = args.first().is_some_and(|arg| {
        Path::new(arg)
            .file_name()
            .is_some_and(|name| name.to_string_lossy() == "codex")
    });
    executable_is_codex && args.iter().any(|arg| arg == "app-server")
}

fn signal(pid: u32, signal: Signal) -> Result<(), String> {
    match kill(Pid::from_raw(pid as i32), signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(e) => Err(format!("failed to signal pid {pid}: {e}")),
    }
}

fn wait_for_remote_to_stop(
    socket: &Path,
    proc_root: &Path,
    pids: &[u32],
    timeout: Duration,
) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let socket_stopped = find_socket_inode(socket, proc_root)?.is_none();
        let processes_stopped = pids.iter().all(|pid| {
            read_cmdline(proc_root, *pid)
                .map(|args| !is_codex_app_server(&args))
                .unwrap_or(true)
        });
        if socket_stopped && processes_stopped {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn remove_stale_socket(socket: &Path) -> Result<(), String> {
    if socket.exists() {
        std::fs::remove_file(socket)
            .map_err(|e| format!("cannot remove stale socket {}: {e}", socket.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        find_socket_inode, find_socket_owners, is_codex_app_server, list_codex_app_server_pids,
        list_codex_desktop_pids, read_cmdline, stop_remote_at, wait_for_remote_to_stop,
    };
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::time::Duration;

    fn fixture(name: &str) -> std::path::PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "codex-switch-process-{}-{}-{suffix}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(base.join("net")).unwrap();
        base
    }

    #[test]
    fn finds_remote_socket_inode_and_owner() {
        let proc_root = fixture("owner");
        let socket = Path::new("/home/test/.codex/app-server-control/app-server-control.sock");
        std::fs::write(
            proc_root.join("net/unix"),
            format!(
                "Num RefCount Protocol Flags Type St Inode Path\n000: 2 0 10000 1 01 4242 {}\n",
                socket.display()
            ),
        )
        .unwrap();
        std::fs::create_dir_all(proc_root.join("123/fd")).unwrap();
        symlink("socket:[4242]", proc_root.join("123/fd/7")).unwrap();
        std::fs::write(
            proc_root.join("123/cmdline"),
            b"/usr/bin/codex\0-c\0features.code_mode_host=true\0app-server\0--listen\0unix://\0",
        )
        .unwrap();

        assert_eq!(find_socket_inode(socket, &proc_root).unwrap(), Some(4242));
        assert_eq!(find_socket_owners(4242, &proc_root).unwrap(), vec![123]);
        assert!(is_codex_app_server(&read_cmdline(&proc_root, 123).unwrap()));
        std::fs::remove_dir_all(proc_root).unwrap();
    }

    #[test]
    fn only_accepts_codex_app_server_commands() {
        assert!(is_codex_app_server(&[
            "/usr/bin/codex".into(),
            "app-server".into(),
            "--listen".into()
        ]));
        assert!(!is_codex_app_server(&[
            "/usr/bin/codex".into(),
            "exec".into()
        ]));
        assert!(!is_codex_app_server(&[
            "/usr/bin/not-codex".into(),
            "app-server".into()
        ]));
    }

    #[test]
    fn discovers_nul_separated_app_server_without_false_positives() {
        let proc_root = fixture("app-server-pids");
        for (pid, cmdline) in [
            (
                123,
                b"/opt/codex/bin/codex\0app-server\0--remote-control\0".as_slice(),
            ),
            (
                124,
                b"/bin/sh\0-c\0codex app-server --remote-control\0".as_slice(),
            ),
            (125, b"/usr/bin/codex\0exec\0--help\0".as_slice()),
        ] {
            std::fs::create_dir_all(proc_root.join(pid.to_string())).unwrap();
            std::fs::write(proc_root.join(format!("{pid}/cmdline")), cmdline).unwrap();
        }

        assert_eq!(list_codex_app_server_pids(&proc_root), vec![123]);
        std::fs::remove_dir_all(proc_root).unwrap();
    }

    #[test]
    fn desktop_detection_only_checks_the_executable() {
        let proc_root = fixture("desktop-pids");
        for (pid, cmdline) in [
            (
                200,
                b"/opt/codex-desktop/codex-desktop\0--flag\0".as_slice(),
            ),
            (
                201,
                b"/bin/sh\0-c\0/opt/codex-desktop/codex-desktop\0".as_slice(),
            ),
        ] {
            std::fs::create_dir_all(proc_root.join(pid.to_string())).unwrap();
            std::fs::write(proc_root.join(format!("{pid}/cmdline")), cmdline).unwrap();
        }

        assert_eq!(list_codex_desktop_pids(&proc_root), vec![200]);
        std::fs::remove_dir_all(proc_root).unwrap();
    }

    #[test]
    fn missing_socket_and_process_are_not_reported_active() {
        let proc_root = fixture("missing");
        std::fs::write(
            proc_root.join("net/unix"),
            "Num RefCount Protocol Flags Type St Inode Path\n",
        )
        .unwrap();
        let socket = proc_root.join("stale.sock");
        std::fs::write(&socket, []).unwrap();

        assert_eq!(find_socket_inode(&socket, &proc_root).unwrap(), None);
        assert!(wait_for_remote_to_stop(&socket, &proc_root, &[999], Duration::ZERO).unwrap());
        stop_remote_at(&socket, &proc_root).unwrap();
        assert!(!socket.exists());
        std::fs::remove_dir_all(proc_root).unwrap();
    }

    #[test]
    fn wait_detects_a_socketless_app_server_process() {
        let proc_root = fixture("wait-process");
        std::fs::write(
            proc_root.join("net/unix"),
            "Num RefCount Protocol Flags Type St Inode Path\n",
        )
        .unwrap();
        std::fs::create_dir_all(proc_root.join("321")).unwrap();
        std::fs::write(
            proc_root.join("321/cmdline"),
            b"/usr/bin/codex\0app-server\0--remote-control\0",
        )
        .unwrap();

        assert!(!wait_for_remote_to_stop(
            Path::new("/missing.sock"),
            &proc_root,
            &[321],
            Duration::ZERO
        )
        .unwrap());
        std::fs::remove_dir_all(proc_root).unwrap();
    }
}

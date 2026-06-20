use crate::data::Context;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::fs::File;
use std::io::{BufRead, BufReader};

/// Find PIDs of Codex desktop processes.
fn list_codex_desktop_pids() -> Vec<u32> {
    let mut pids = Vec::new();

    let proc_dir = std::path::PathBuf::from("/proc");
    let entries = match std::fs::read_dir(&proc_dir) {
        Ok(e) => e,
        Err(_) => return pids,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        // Only consider numeric directories (PIDs)
        if !path.is_dir()
            || !path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .parse::<u32>()
                .is_ok()
        {
            continue;
        }

        let cmdline_path = path.join("cmdline");
        let file = match File::open(&cmdline_path) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let reader = BufReader::new(file);
        let cmdline = reader
            .lines()
            .next()
            .and_then(|l| l.ok())
            .unwrap_or_default();

        if cmdline.contains("/opt/codex-desktop/")
            || cmdline.contains("codex app-server --remote-control")
        {
            let pid = path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .parse::<u32>()
                .unwrap();
            pids.push(pid);
        }
    }

    pids.sort();
    pids.dedup();
    pids
}

pub fn kill_codex_desktop(_ctx: &Context) {
    let pids = list_codex_desktop_pids();

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

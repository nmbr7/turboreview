//! Listing running processes for the "attach to process" debug launch mode.

/// A running process: pid + command (basename of the executable / argv0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: i64,
    pub command: String,
}

/// List running processes via `ps`. Returns them sorted by pid descending
/// (newest-ish first). Errors are surfaced as an empty list by the caller.
pub fn list_processes() -> std::io::Result<Vec<ProcInfo>> {
    // `comm` is the executable path; we keep its basename for readability but
    // fall back to the full string when there's no separator.
    let out = std::process::Command::new("ps")
        .args(["-axo", "pid=,comm="])
        .output()?;
    Ok(parse_ps(&String::from_utf8_lossy(&out.stdout)))
}

/// Parse `ps -axo pid=,comm=` output: each line is `<pid> <command>`.
pub fn parse_ps(text: &str) -> Vec<ProcInfo> {
    let mut out: Vec<ProcInfo> = text
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid_str, rest) = line.split_once(char::is_whitespace)?;
            let pid = pid_str.trim().parse::<i64>().ok()?;
            let cmd = rest.trim();
            let base = cmd.rsplit('/').next().unwrap_or(cmd);
            Some(ProcInfo {
                pid,
                command: base.to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.pid.cmp(&a.pid));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ps_extracts_pid_and_basename() {
        let text = "    1 /sbin/launchd\n  546 /usr/libexec/logd\n 9999 myapp\n";
        let procs = parse_ps(text);
        // Sorted by pid desc.
        assert_eq!(procs[0], ProcInfo { pid: 9999, command: "myapp".into() });
        assert_eq!(procs[1].pid, 546);
        assert_eq!(procs[1].command, "logd");
        assert_eq!(procs[2].command, "launchd");
    }

    #[test]
    fn parse_ps_skips_malformed_lines() {
        let procs = parse_ps("garbage\n  42 /bin/zsh\n\n");
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].pid, 42);
        assert_eq!(procs[0].command, "zsh");
    }
}

//! Process tree, memory (RSS/PSS) and CPU time read from `/proc` (D3).
//!
//! Every function takes the proc root as a parameter so the tests can point
//! it at a fake tree (a folder with `<pid>/stat`, `<pid>/status` and
//! `<pid>/smaps_rollup`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Fields of `/proc/<pid>/stat` the bench needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub pid: u32,
    pub ppid: u32,
    /// `utime + stime`, in clock ticks.
    pub cpu_ticks: u64,
}

/// Parses one `stat` line. The command name (field 2) may contain spaces and
/// parentheses, so the fields are counted from the last `)`.
pub fn parse_stat(line: &str) -> Option<Stat> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    let pid = line[..open].trim().parse().ok()?;
    // After ")": state(3) ppid(4) ... utime(14) stime(15).
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    let field = |n: usize| rest.get(n - 3).copied();
    let ppid = field(4)?.parse().ok()?;
    let utime: u64 = field(14)?.parse().ok()?;
    let stime: u64 = field(15)?.parse().ok()?;
    Some(Stat {
        pid,
        ppid,
        cpu_ticks: utime + stime,
    })
}

/// Every readable process under `root`.
pub fn all_stats(root: &Path) -> Vec<Stat> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|entry| fs::read_to_string(entry.path().join("stat")).ok())
        .filter_map(|line| parse_stat(&line))
        .collect()
}

/// `root_pid` and all its descendants (by parent pid), sorted.
pub fn tree(root: &Path, root_pid: u32) -> Vec<u32> {
    descendants(&all_stats(root), root_pid, true)
}

/// Descendants of `root_pid` in `stats`, optionally including itself.
pub fn descendants(stats: &[Stat], root_pid: u32, include_root: bool) -> Vec<u32> {
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for stat in stats {
        children.entry(stat.ppid).or_default().push(stat.pid);
    }
    let mut found = BTreeSet::new();
    let mut pending = vec![root_pid];
    while let Some(pid) = pending.pop() {
        if let Some(kids) = children.get(&pid) {
            for &kid in kids {
                if kid != root_pid && found.insert(kid) {
                    pending.push(kid);
                }
            }
        }
    }
    if include_root && stats.iter().any(|stat| stat.pid == root_pid) {
        found.insert(root_pid);
    }
    found.into_iter().collect()
}

/// A `kB` field (`Rss:`, `Pss:`, `VmRSS:`) of a `status`/`smaps_rollup` text.
pub fn kb_field(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix(name)?.strip_prefix(':')?;
        value.split_whitespace().next()?.parse().ok()
    })
}

/// Memory of a set of processes, in kB.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    /// `VmRSS` of the first (main) process.
    pub rss_main_kb: u64,
    /// Sum of `Rss` over the tree (counts shared pages once per process).
    pub rss_tree_kb: u64,
    /// Sum of `Pss` over the tree (shared pages split between processes).
    pub pss_tree_kb: u64,
    /// Processes that could be read.
    pub processes: usize,
}

/// RSS of `main_pid` and RSS/PSS summed over `pids` (D3). Processes that
/// vanished between the listing and the read are skipped.
pub fn memory(root: &Path, main_pid: u32, pids: &[u32]) -> Memory {
    let mut memory = Memory {
        rss_main_kb: fs::read_to_string(root.join(main_pid.to_string()).join("status"))
            .ok()
            .and_then(|text| kb_field(&text, "VmRSS"))
            .unwrap_or(0),
        ..Memory::default()
    };
    for pid in pids {
        let Ok(text) = fs::read_to_string(root.join(pid.to_string()).join("smaps_rollup")) else {
            continue;
        };
        memory.rss_tree_kb += kb_field(&text, "Rss").unwrap_or(0);
        memory.pss_tree_kb += kb_field(&text, "Pss").unwrap_or(0);
        memory.processes += 1;
    }
    memory
}

/// CPU ticks per pid of a set of processes (processes that exited are absent).
pub fn cpu_ticks(root: &Path, pids: &[u32]) -> BTreeMap<u32, u64> {
    pids.iter()
        .filter_map(|pid| {
            let line = fs::read_to_string(root.join(pid.to_string()).join("stat")).ok()?;
            parse_stat(&line).map(|stat| (*pid, stat.cpu_ticks))
        })
        .collect()
}

/// Ticks spent between two readings; processes born in between count from 0.
pub fn ticks_between(before: &BTreeMap<u32, u64>, after: &BTreeMap<u32, u64>) -> u64 {
    after
        .iter()
        .map(|(pid, ticks)| ticks.saturating_sub(before.get(pid).copied().unwrap_or(0)))
        .sum()
}

use crate::cache;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
#[repr(C)]
struct Timeval {
    sec: i64,
    usec: i32,
    _pad: i32,
}

#[cfg(not(target_os = "macos"))]
#[repr(C)]
struct Timeval {
    sec: i64,
    usec: i64,
}

#[repr(C)]
struct Rusage {
    utime: Timeval,
    stime: Timeval,
    maxrss: i64,
    rest: [i64; 14],
}

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut Rusage) -> i32;
}

pub struct CpuSnapshot {
    pub cpu_ms: u64,
    pub maxrss_bytes: i64,
}

pub fn children_cpu() -> CpuSnapshot {
    let mut r: Rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { getrusage(-1, &mut r) };
    if rc != 0 {
        return CpuSnapshot {
            cpu_ms: 0,
            maxrss_bytes: 0,
        };
    }
    let cpu_ms = (r.utime.sec + r.stime.sec).max(0) as u64 * 1000
        + ((r.utime.usec as i64 + r.stime.usec as i64).max(0) / 1000) as u64;
    let maxrss_bytes = if cfg!(target_os = "macos") {
        r.maxrss
    } else {
        r.maxrss * 1024
    };
    CpuSnapshot {
        cpu_ms,
        maxrss_bytes,
    }
}

fn ledger_path() -> PathBuf {
    cache::root().join("stats.tsv")
}

pub fn record(kind: &str, a: u64, b: u64, c: i64, label: &str, cmd: &str) {
    let _ = fs::create_dir_all(cache::root());
    let line = format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        kind,
        cache::now(),
        a,
        b,
        c,
        label,
        cmd.replace(['\t', '\n'], " ")
    );
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger_path())
    {
        let _ = f.write_all(line.as_bytes());
    }
    trim_if_large();
}

fn trim_if_large() {
    let path = ledger_path();
    let too_big = fs::metadata(&path)
        .map(|m| m.len() > 512 * 1024)
        .unwrap_or(false);
    if !too_big {
        return;
    }
    if let Ok(data) = fs::read_to_string(&path) {
        let lines: Vec<&str> = data.lines().collect();
        let keep = &lines[lines.len().saturating_sub(2000)..];
        let _ = fs::write(&path, keep.join("\n") + "\n");
    }
}

#[derive(Default)]
pub struct PerCmd {
    pub spec_runs: u64,
    pub spec_cpu_ms: u64,
    pub hits: u64,
    pub saved_ms: u64,
}

#[derive(Default)]
pub struct Summary {
    pub first_ts: u64,
    pub spec_runs: u64,
    pub spec_wall_ms: u64,
    pub spec_cpu_ms: u64,
    pub peak_rss_bytes: i64,
    pub hits: u64,
    pub saved_ms: u64,
    pub misses: u64,
    pub miss_overhead_ms: u64,
    pub by_cmd: HashMap<String, PerCmd>,
}

pub fn aggregate(data: &str) -> Summary {
    let mut s = Summary::default();
    for line in data.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let ts: u64 = f[1].parse().unwrap_or(0);
        let a: u64 = f[2].parse().unwrap_or(0);
        let b: u64 = f[3].parse().unwrap_or(0);
        let c: i64 = f[4].parse().unwrap_or(0);
        let cmd = f[6];
        if s.first_ts == 0 || (ts > 0 && ts < s.first_ts) {
            s.first_ts = ts;
        }
        let per = s.by_cmd.entry(cmd.to_string()).or_default();
        match f[0] {
            "spec" => {
                s.spec_runs += 1;
                s.spec_wall_ms += a;
                s.spec_cpu_ms += b;
                s.peak_rss_bytes = s.peak_rss_bytes.max(c);
                per.spec_runs += 1;
                per.spec_cpu_ms += b;
            }
            "serve" => {
                s.hits += 1;
                s.saved_ms += a;
                per.hits += 1;
                per.saved_ms += a;
            }
            "miss" => {
                s.misses += 1;
                s.miss_overhead_ms += a;
            }
            _ => {}
        }
    }
    s
}

pub fn print_summary() {
    let data = fs::read_to_string(ledger_path()).unwrap_or_default();
    if data.is_empty() {
        println!("specsh stats: no events recorded yet");
        return;
    }
    let s = aggregate(&data);
    let age_h = (cache::now().saturating_sub(s.first_ts)) as f64 / 3600.0;
    println!(
        "specsh stats (last {:.1}h, ledger {})",
        age_h,
        ledger_path().display()
    );
    println!(
        "  speculation: {} runs, {:.1}s wall, {:.1}s cpu (niced), peak child rss {:.0} MB",
        s.spec_runs,
        s.spec_wall_ms as f64 / 1000.0,
        s.spec_cpu_ms as f64 / 1000.0,
        s.peak_rss_bytes as f64 / 1_048_576.0
    );
    println!(
        "  served: {} hits, {:.1}s of foreground waiting avoided",
        s.hits,
        s.saved_ms as f64 / 1000.0
    );
    println!(
        "  misses: {} eligible commands ran live ({:.0}ms total lookup overhead)",
        s.misses, s.miss_overhead_ms as f64
    );
    let denom = s.hits + s.misses;
    if denom > 0 {
        println!(
            "  hit rate: {:.0}% of eligible invocations",
            s.hits as f64 / denom as f64 * 100.0
        );
    }
    let net = s.saved_ms as f64 - s.spec_cpu_ms as f64;
    println!(
        "  net: {:.1}s foreground saved vs {:.1}s background cpu spent ({}{:.1}s)",
        s.saved_ms as f64 / 1000.0,
        s.spec_cpu_ms as f64 / 1000.0,
        if net >= 0.0 { "+" } else { "" },
        net / 1000.0
    );
    let mut rows: Vec<(&String, &PerCmd)> = s
        .by_cmd
        .iter()
        .filter(|(_, p)| p.spec_runs > 0 || p.hits > 0)
        .collect();
    rows.sort_by_key(|(_, p)| std::cmp::Reverse(p.saved_ms));
    if !rows.is_empty() {
        println!("  by command:");
        for (cmd, p) in rows.iter().take(10) {
            println!(
                "    {:<40} {} hits / {} runs, saved {:.1}s, cpu {:.1}s",
                cmd,
                p.hits,
                p.spec_runs,
                p.saved_ms as f64 / 1000.0,
                p.spec_cpu_ms as f64 / 1000.0
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_payoff_and_loss() {
        let data = "spec\t100\t4000\t3500\t1048576\tcached\tcargo test\n\
                    spec\t200\t2000\t1800\t2097152\tnegative\tnpm test\n\
                    serve\t300\t4000\t10\t0\thit\tcargo test\n\
                    miss\t400\t12\t0\t0\tmiss\tcargo test\n";
        let s = aggregate(data);
        assert_eq!(s.spec_runs, 2);
        assert_eq!(s.spec_cpu_ms, 5300);
        assert_eq!(s.hits, 1);
        assert_eq!(s.saved_ms, 4000);
        assert_eq!(s.misses, 1);
        assert_eq!(s.peak_rss_bytes, 2097152);
        assert_eq!(s.first_ts, 100);
        let per = &s.by_cmd["cargo test"];
        assert_eq!(per.hits, 1);
        assert_eq!(per.spec_runs, 1);
    }

    #[test]
    fn children_cpu_reads_rusage() {
        let _ = std::process::Command::new("/bin/sh")
            .args(["-c", "true"])
            .status();
        let snap = children_cpu();
        assert!(snap.maxrss_bytes >= 0);
    }
}

use crate::{cache, clone, eligible, hash, sandbox, stats, taint};
use std::path::Path;
use std::time::Duration;

pub struct Outcome {
    pub cmd: String,
    pub action: &'static str,
    pub detail: String,
}

const SUSPICIOUS: &[&str] = &[
    "could not resolve",
    "network is unreachable",
    "no route to host",
    "getaddrinfo",
    "eai_again",
    "name resolution",
    "operation not permitted",
    "read-only file system",
];

fn smells_sandboxed(res: &sandbox::RunResult) -> Option<&'static str> {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&res.stdout).to_lowercase(),
        String::from_utf8_lossy(&res.stderr).to_lowercase()
    );
    SUSPICIOUS.iter().find(|n| text.contains(**n)).copied()
}

pub fn run_pipeline(
    cwd: &Path,
    candidates: &[String],
    max_runs: usize,
    timeout: Duration,
) -> Vec<Outcome> {
    let cwd_s = cwd.to_string_lossy().to_string();
    let mut out = Vec::new();
    let mut runs = 0;
    for cmd in candidates {
        if runs >= max_runs {
            break;
        }
        let o = |action, detail: String| Outcome {
            cmd: cmd.clone(),
            action,
            detail,
        };
        if let Err(reason) = eligible::eligible(cmd) {
            out.push(o("skip", reason));
            continue;
        }
        let head = cmd.split_whitespace().next().unwrap_or("");
        if !eligible::marker_present(cwd, head) {
            out.push(o("skip", "no project marker in cwd".to_string()));
            continue;
        }
        if cache::negative_has(&cwd_s, cmd) {
            out.push(o("skip", "negative cache".to_string()));
            continue;
        }
        let tree = match hash::tree_hash(cwd) {
            Ok(t) => t,
            Err(e) => {
                out.push(o("skip", format!("tree hash failed: {e}")));
                continue;
            }
        };
        if let Some(e) = cache::load(&cwd_s, cmd)
            && e.fresh(&tree)
        {
            out.push(o("skip", "already cached and fresh".to_string()));
            continue;
        }
        runs += 1;
        let started_at = taint::local_timestamp();
        let cpu_before = stats::children_cpu();
        let work = match clone::WorkClone::create(cwd, false) {
            Ok(w) => w,
            Err(e) => {
                out.push(o("skip", format!("clone failed: {e}")));
                continue;
            }
        };
        let res = match sandbox::run_opts(&work.path, cmd, timeout, true, true) {
            Ok(r) => r,
            Err(e) => {
                out.push(o("skip", format!("spawn failed: {e}")));
                continue;
            }
        };
        drop(work);
        let cpu_after = stats::children_cpu();
        let spec_cpu_ms = cpu_after.cpu_ms.saturating_sub(cpu_before.cpu_ms);

        let mut denials = Vec::new();
        if res.exit_code != 0
            && let Some(start) = &started_at
        {
            denials = taint::denials_since(start, true);
        }
        let outcome = if !denials.is_empty() {
            let _ = cache::negative_add(&cwd_s, cmd);
            out.push(o(
                "negative",
                format!("tainted, {} sandbox denial(s)", denials.len()),
            ));
            "negative"
        } else if res.timed_out {
            out.push(o("skip", format!("timed out after {timeout:?}")));
            "timeout"
        } else if let Some(needle) = smells_sandboxed(&res) {
            out.push(o("skip", format!("output mentions '{needle}'")));
            "suspicious"
        } else {
            let entry = cache::Entry {
                cwd: cwd_s.clone(),
                cmd: cmd.clone(),
                tree,
                exit_code: res.exit_code,
                created: cache::now(),
                duration_ms: res.duration.as_millis() as u64,
                stdout: res.stdout,
                stderr: res.stderr,
            };
            match cache::store(&entry) {
                Ok(()) => {
                    out.push(o(
                        "cached",
                        format!(
                            "exit {} in {:.1}s",
                            entry.exit_code,
                            entry.duration_ms as f64 / 1000.0
                        ),
                    ));
                    "cached"
                }
                Err(e) => {
                    out.push(o("skip", format!("store failed: {e}")));
                    "store-failed"
                }
            }
        };
        stats::record(
            "spec",
            res.duration.as_millis() as u64,
            spec_cpu_ms,
            cpu_after.maxrss_bytes,
            outcome,
            cmd,
        );
    }
    out
}

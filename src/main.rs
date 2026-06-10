use std::io::Write;
use std::process::exit;
use std::time::Duration;

mod clone;
mod sandbox;
mod taint;

const TAINTED_EXIT: i32 = 113;

fn main() {
    if !cfg!(target_os = "macos") {
        eprintln!("specsh: macOS only for now (clonefile + sandbox-exec)");
        exit(1);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut timeout = 300u64;
    let mut keep = false;
    let mut cmd_parts: Vec<String> = Vec::new();
    let mut i = 0;
    let mut subcommand = None;
    while i < args.len() {
        match args[i].as_str() {
            "run" | "profile" if subcommand.is_none() => subcommand = Some(args[i].clone()),
            "--timeout" => {
                i += 1;
                timeout = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage("--timeout requires seconds"));
            }
            "--keep" => keep = true,
            "--" => {
                cmd_parts.extend(args[i + 1..].iter().cloned());
                break;
            }
            "-h" | "--help" => usage(""),
            other => {
                if subcommand.is_some() {
                    cmd_parts.push(other.to_string());
                } else {
                    usage(&format!("unknown argument: {}", other));
                }
            }
        }
        i += 1;
    }

    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("specsh: cannot resolve cwd: {}", e);
        exit(1);
    });

    match subcommand.as_deref() {
        Some("profile") => {
            print!("{}", sandbox::profile(&cwd));
        }
        Some("run") => {
            if cmd_parts.is_empty() {
                usage("run requires a command");
            }
            let command = cmd_parts.join(" ");
            let started_at = taint::local_timestamp();
            let work = clone::WorkClone::create(&cwd, keep).unwrap_or_else(|e| {
                eprintln!("specsh: clone failed: {}", e);
                exit(1);
            });
            let result = sandbox::run(&work.path, &command, Duration::from_secs(timeout))
                .unwrap_or_else(|e| {
                    eprintln!("specsh: sandbox spawn failed: {}", e);
                    exit(1);
                });

            let mut tainted = Vec::new();
            if result.exit_code != 0 {
                if let Some(start) = started_at {
                    tainted = taint::sandbox_denials_since(&start);
                }
            }

            std::io::stdout().write_all(&result.stdout).ok();
            std::io::stderr().write_all(&result.stderr).ok();
            eprintln!(
                "specsh: exit {} in {:.2}s (clone {}{})",
                result.exit_code,
                result.duration.as_secs_f64(),
                if keep {
                    work.path.display().to_string()
                } else {
                    "discarded".to_string()
                },
                if result.timed_out { ", TIMED OUT" } else { "" }
            );
            if !tainted.is_empty() {
                eprintln!(
                    "specsh: TAINTED, {} sandbox denial(s) during run; this failure may be sandbox-induced, not real:",
                    tainted.len()
                );
                for line in tainted.iter().take(5) {
                    eprintln!("  {}", line);
                }
                exit(TAINTED_EXIT);
            }
            exit(result.exit_code);
        }
        _ => usage(""),
    }
}

fn usage(err: &str) -> ! {
    if !err.is_empty() {
        eprintln!("specsh: {}", err);
    }
    eprintln!("usage: specsh run [--timeout SECS] [--keep] -- <command>");
    eprintln!("       specsh profile");
    eprintln!("  runs <command> against a copy-on-write clone of the cwd inside a");
    eprintln!("  no-network sandbox, prints captured output, and discards the clone");
    eprintln!("  exit {}: result tainted by a sandbox denial", TAINTED_EXIT);
    exit(2);
}

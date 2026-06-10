use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, exit};
use std::time::Duration;

mod cache;
mod clone;
mod eligible;
mod hash;
mod history;
mod predict;
mod sandbox;
mod spec;
mod stats;
mod taint;

const TAINTED_EXIT: i32 = 113;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..]),
        Some("exec") => cmd_exec(&args[1..]),
        Some("speculate") => cmd_speculate(&args[1..]),
        Some("predict") => cmd_predict(&args[1..]),
        Some("profile") => {
            print!("{}", sandbox::profile(&cwd()));
        }
        Some("init") => cmd_init(&args[1..]),
        Some("status") => cmd_status(),
        Some("stats") => stats::print_summary(),
        _ => usage(""),
    }
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("specsh: cannot resolve cwd: {e}");
        exit(1);
    })
}

fn split_dashdash(args: &[String]) -> (Vec<String>, Vec<String>) {
    match args.iter().position(|a| a == "--") {
        Some(i) => (args[..i].to_vec(), args[i + 1..].to_vec()),
        None => (args.to_vec(), Vec::new()),
    }
}

fn flag_value(flags: &[String], name: &str) -> Option<String> {
    flags
        .iter()
        .position(|a| a == name)
        .and_then(|i| flags.get(i + 1).cloned())
}

fn cmd_run(args: &[String]) -> ! {
    let (flags, cmd_parts) = split_dashdash(args);
    let timeout: u64 = flag_value(&flags, "--timeout")
        .map(|v| {
            v.parse()
                .unwrap_or_else(|_| usage("--timeout requires seconds"))
        })
        .unwrap_or(300);
    let keep = flags.iter().any(|a| a == "--keep");
    if cmd_parts.is_empty() {
        usage("run requires a command after --");
    }
    let command = cmd_parts.join(" ");
    let started_at = taint::local_timestamp();
    let work = clone::WorkClone::create(&cwd(), keep).unwrap_or_else(|e| {
        eprintln!("specsh: clone failed: {e}");
        exit(1);
    });
    let result =
        sandbox::run(&work.path, &command, Duration::from_secs(timeout)).unwrap_or_else(|e| {
            eprintln!("specsh: sandbox spawn failed: {e}");
            exit(1);
        });

    let mut tainted = Vec::new();
    if result.exit_code != 0
        && let Some(start) = started_at
    {
        tainted = taint::denials_since(&start, false);
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
            eprintln!("  {line}");
        }
        exit(TAINTED_EXIT);
    }
    exit(result.exit_code);
}

fn cmd_exec(args: &[String]) -> ! {
    let (_, argv) = split_dashdash(args);
    if argv.is_empty() {
        usage("exec requires a command after --");
    }
    let cmd_str = cache::join_args(&argv);
    let dir = cwd();
    let dir_s = dir.to_string_lossy().to_string();
    if eligible::eligible(&cmd_str).is_ok() && eligible::marker_present(&dir, &argv[0]) {
        let lookup = std::time::Instant::now();
        if let Ok(tree) = hash::tree_hash(&dir)
            && let Some(e) = cache::load(&dir_s, &cmd_str)
            && e.fresh(&tree)
        {
            std::io::stdout().write_all(&e.stdout).ok();
            std::io::stderr().write_all(&e.stderr).ok();
            eprintln!(
                "specsh: served speculated result from {}s ago, saved ~{:.1}s",
                e.age(),
                e.duration_ms as f64 / 1000.0
            );
            stats::record("serve", e.duration_ms, e.age(), 0, "hit", &cmd_str);
            exit(e.exit_code);
        }
        stats::record(
            "miss",
            lookup.elapsed().as_millis() as u64,
            0,
            0,
            "miss",
            &cmd_str,
        );
    }
    let err = Command::new(&argv[0]).args(&argv[1..]).exec();
    eprintln!("specsh: failed to exec {}: {}", argv[0], err);
    exit(127);
}

fn cmd_speculate(args: &[String]) {
    let (flags, _) = split_dashdash(args);
    let after = flag_value(&flags, "--after");
    let only = flag_value(&flags, "--only");
    let max_runs: usize = flag_value(&flags, "--max")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let timeout: u64 = flag_value(&flags, "--timeout")
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);

    let Some(_lock) = cache::Lock::acquire() else {
        eprintln!("specsh: speculation already in progress, skipping");
        return;
    };

    let candidates: Vec<String> = match only {
        Some(c) => vec![c],
        None => {
            let cmds = history::parse(&history::default_path()).unwrap_or_default();
            let model = predict::Model::build(&cmds);
            model
                .candidates(after.as_deref().unwrap_or(""), 6)
                .into_iter()
                .map(|c| c.cmd)
                .collect()
        }
    };
    if candidates.is_empty() {
        eprintln!("specsh: no candidates");
        return;
    }
    let outcomes = spec::run_pipeline(&cwd(), &candidates, max_runs, Duration::from_secs(timeout));
    for o in &outcomes {
        eprintln!("specsh: [{}] {} ({})", o.action, o.cmd, o.detail);
    }
}

fn cmd_predict(args: &[String]) -> ! {
    let (flags, _) = split_dashdash(args);
    let after = flag_value(&flags, "--after").unwrap_or_default();
    let cmds = history::parse(&history::default_path()).unwrap_or_else(|e| {
        eprintln!("specsh: cannot read history: {e}");
        exit(1);
    });
    let model = predict::Model::build(&cmds);
    let dir = cwd();
    println!("after: {after:?}");
    for c in model.candidates(&after, 6) {
        let verdict = match eligible::eligible(&c.cmd) {
            Ok(()) => {
                let head = c.cmd.split_whitespace().next().unwrap_or("");
                if eligible::marker_present(&dir, head) {
                    "ELIGIBLE".to_string()
                } else {
                    "skip: no project marker".to_string()
                }
            }
            Err(r) => format!("skip: {r}"),
        };
        println!(
            "  {:>4}x  {:<9} {:<28} {}",
            c.count, c.source, verdict, c.cmd
        );
    }
    exit(0);
}

fn cmd_init(args: &[String]) {
    let shell = flag_value(args, "--shell").unwrap_or_else(|| "fish".to_string());
    match shell.as_str() {
        "fish" => {
            println!("function __specsh_postexec --on-event fish_postexec");
            println!("    test -n \"$argv[1]\"; or return");
            println!(
                "    command specsh speculate --after \"$argv[1]\" </dev/null >/dev/null 2>&1 &"
            );
            println!("    disown 2>/dev/null");
            println!("end");
            for head in eligible::WRAPPED_HEADS {
                println!();
                println!("function {head} --wraps {head}");
                println!("    command specsh exec -- {head} $argv");
                println!("end");
            }
        }
        "zsh" => {
            println!("__specsh_preexec() {{ __specsh_last=\"$1\"; }}");
            println!("__specsh_precmd() {{");
            println!("    if [ -n \"$__specsh_last\" ]; then");
            println!(
                "        command specsh speculate --after \"$__specsh_last\" </dev/null >/dev/null 2>&1 &!"
            );
            println!("        __specsh_last=\"\"");
            println!("    fi");
            println!("}}");
            println!("autoload -Uz add-zsh-hook");
            println!("add-zsh-hook preexec __specsh_preexec");
            println!("add-zsh-hook precmd __specsh_precmd");
            for head in eligible::WRAPPED_HEADS {
                println!();
                println!("{head}() {{ command specsh exec -- {head} \"$@\"; }}");
            }
        }
        "bash" => {
            println!("__specsh_hook() {{");
            println!("    local last");
            println!(
                "    last=$(HISTTIMEFORMAT= builtin history 1 2>/dev/null | sed 's/^ *[0-9]* *//')"
            );
            println!("    if [ -n \"$last\" ] && [ \"$last\" != \"$__specsh_prev\" ]; then");
            println!("        __specsh_prev=\"$last\"");
            println!(
                "        {{ command specsh speculate --after \"$last\" </dev/null >/dev/null 2>&1 & disown; }} 2>/dev/null"
            );
            println!("    fi");
            println!("}}");
            println!("case \"$PROMPT_COMMAND\" in");
            println!("    *__specsh_hook*) ;;");
            println!(
                "    *) PROMPT_COMMAND=\"__specsh_hook${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\" ;;"
            );
            println!("esac");
            for head in eligible::WRAPPED_HEADS {
                println!();
                println!("{head}() {{ command specsh exec -- {head} \"$@\"; }}");
            }
        }
        other => usage(&format!("unknown shell '{other}' (fish, zsh, bash)")),
    }
}

fn cmd_status() {
    let entries = cache::list();
    let negatives = cache::negative_list();
    println!(
        "specsh cache at {} ({} results, {} negative)",
        cache::root().display(),
        entries.len(),
        negatives.len()
    );
    for e in &entries {
        let state = match hash::tree_hash(std::path::Path::new(&e.cwd)) {
            Ok(t) if e.fresh(&t) => "fresh",
            _ => "stale",
        };
        println!(
            "  [{}] exit {:>3}  {:>5.1}s  {:>5}s ago  {}  {}",
            state,
            e.exit_code,
            e.duration_ms as f64 / 1000.0,
            e.age(),
            e.cwd,
            e.cmd
        );
    }
    if !negatives.is_empty() {
        println!("negative (non-speculable):");
        for n in &negatives {
            println!("  {n}");
        }
    }
}

fn usage(err: &str) -> ! {
    if !err.is_empty() {
        eprintln!("specsh: {err}");
    }
    eprintln!("usage: specsh exec -- <command>             serve speculated result or exec live");
    eprintln!("       specsh speculate [--after CMD] [--only CMD] [--max N] [--timeout SECS]");
    eprintln!("       specsh predict [--after CMD]         show predictions and eligibility");
    eprintln!("       specsh run [--timeout SECS] [--keep] -- <command>");
    eprintln!("       specsh init [--shell fish|zsh|bash]  print shell integration (source it)");
    eprintln!("       specsh status                        show cache contents");
    eprintln!("       specsh stats                         speculation cost vs payoff");
    eprintln!("       specsh profile                       print sandbox profile for cwd");
    eprintln!("  exit {TAINTED_EXIT}: result tainted by a sandbox denial");
    exit(2);
}

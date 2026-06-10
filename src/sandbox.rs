use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct RunResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration: Duration,
}

pub fn profile(clone_path: &Path) -> String {
    let mut allow_writes = vec![
        sb_path(clone_path),
        "(subpath \"/dev\")".to_string(),
        "(subpath \"/private/tmp\")".to_string(),
        "(subpath \"/private/var/tmp\")".to_string(),
        "(subpath \"/private/var/folders\")".to_string(),
    ];
    if let Ok(tmp) = std::env::var("TMPDIR") {
        allow_writes.push(sb_path(Path::new(&tmp)));
    }
    format!(
        "(version 1)\n(allow default)\n(deny network*)\n(deny file-write* (subpath \"/\"))\n(allow file-write* {})\n",
        allow_writes.join(" ")
    )
}

fn sb_path(p: &Path) -> String {
    let escaped = p
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("(subpath \"{}\")", escaped)
}

pub fn run(clone_path: &Path, command: &str, timeout: Duration) -> io::Result<RunResult> {
    let start = Instant::now();
    let mut child = Command::new("/usr/bin/sandbox-exec")
        .arg("-p")
        .arg(profile(clone_path))
        .arg("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(clone_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut out_pipe = child.stdout.take().unwrap();
    let mut err_pipe = child.stderr.take().unwrap();
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });

    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    Ok(RunResult {
        stdout: out_reader.join().unwrap_or_default(),
        stderr: err_reader.join().unwrap_or_default(),
        exit_code: status.code().unwrap_or(-1),
        timed_out,
        duration: start.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_workdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("specsh-sbx-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn captures_stdout_and_exit() {
        let d = tmp_workdir();
        let r = run(&d, "echo hot; exit 3", Duration::from_secs(10)).unwrap();
        assert_eq!(String::from_utf8_lossy(&r.stdout), "hot\n");
        assert_eq!(r.exit_code, 3);
        assert!(!r.timed_out);
    }

    #[test]
    fn writes_inside_clone_allowed() {
        let d = tmp_workdir();
        let r = run(&d, "echo x > made.txt && cat made.txt", Duration::from_secs(10)).unwrap();
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "x\n");
    }

    #[test]
    fn writes_outside_clone_denied() {
        let d = tmp_workdir();
        let target = std::env::var("HOME").unwrap() + "/.specsh_deny_probe";
        let r = run(&d, &format!("touch {}", target), Duration::from_secs(10)).unwrap();
        assert_ne!(r.exit_code, 0);
        assert!(!Path::new(&target).exists());
    }

    #[test]
    fn network_denied() {
        let d = tmp_workdir();
        let r = run(
            &d,
            "curl -sS -m 5 https://captive.apple.com",
            Duration::from_secs(10),
        )
        .unwrap();
        assert_ne!(r.exit_code, 0);
    }

    #[test]
    fn timeout_kills() {
        let d = tmp_workdir();
        let r = run(&d, "sleep 30", Duration::from_millis(300)).unwrap();
        assert!(r.timed_out);
    }
}

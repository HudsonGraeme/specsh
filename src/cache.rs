use crate::hash;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TTL_SECS: u64 = 900;

pub struct Entry {
    pub cwd: String,
    pub cmd: String,
    pub tree: String,
    pub exit_code: i32,
    pub created: u64,
    pub duration_ms: u64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Entry {
    pub fn age(&self) -> u64 {
        now().saturating_sub(self.created)
    }

    pub fn fresh(&self, current_tree: &str) -> bool {
        self.tree == current_tree && self.age() <= TTL_SECS
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn root() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("specsh");
        }
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".cache/specsh")
}

fn results_dir() -> PathBuf {
    root().join("results")
}

fn negative_dir() -> PathBuf {
    root().join("negative")
}

pub fn key(cwd: &str, cmd: &str) -> String {
    let mut data = Vec::with_capacity(cwd.len() + cmd.len() + 1);
    data.extend_from_slice(cwd.as_bytes());
    data.push(0);
    data.extend_from_slice(cmd.as_bytes());
    hash::hex128(hash::fnv128(&data))
}

pub fn store(e: &Entry) -> io::Result<()> {
    let dir = results_dir();
    fs::create_dir_all(&dir)?;
    let k = key(&e.cwd, &e.cmd);
    let mut buf = Vec::new();
    buf.extend_from_slice(b"specsh1\n");
    buf.extend_from_slice(format!("exit {}\n", e.exit_code).as_bytes());
    buf.extend_from_slice(format!("created {}\n", e.created).as_bytes());
    buf.extend_from_slice(format!("duration_ms {}\n", e.duration_ms).as_bytes());
    buf.extend_from_slice(format!("tree {}\n", e.tree).as_bytes());
    for (name, bytes) in [
        ("cwd", e.cwd.as_bytes()),
        ("cmd", e.cmd.as_bytes()),
        ("stdout", &e.stdout[..]),
        ("stderr", &e.stderr[..]),
    ] {
        buf.extend_from_slice(format!("{} {}\n", name, bytes.len()).as_bytes());
        buf.extend_from_slice(bytes);
        buf.push(b'\n');
    }
    let tmp = dir.join(format!("{}.tmp", k));
    fs::write(&tmp, &buf)?;
    fs::rename(&tmp, dir.join(k))
}

pub fn load(cwd: &str, cmd: &str) -> Option<Entry> {
    read_entry(&results_dir().join(key(cwd, cmd)))
}

fn read_line(data: &[u8], pos: &mut usize) -> Option<String> {
    let nl = data[*pos..].iter().position(|&b| b == b'\n')? + *pos;
    let s = String::from_utf8_lossy(&data[*pos..nl]).to_string();
    *pos = nl + 1;
    Some(s)
}

fn read_blob(data: &[u8], pos: &mut usize, name: &str) -> Option<Vec<u8>> {
    let hdr = read_line(data, pos)?;
    let n: usize = hdr.strip_prefix(&format!("{} ", name))?.parse().ok()?;
    if *pos + n + 1 > data.len() {
        return None;
    }
    let b = data[*pos..*pos + n].to_vec();
    *pos += n + 1;
    Some(b)
}

pub fn read_entry(path: &Path) -> Option<Entry> {
    let data = fs::read(path).ok()?;
    let mut pos = 0;
    if read_line(&data, &mut pos)? != "specsh1" {
        return None;
    }
    let exit_code = read_line(&data, &mut pos)?.strip_prefix("exit ")?.parse().ok()?;
    let created = read_line(&data, &mut pos)?.strip_prefix("created ")?.parse().ok()?;
    let duration_ms = read_line(&data, &mut pos)?
        .strip_prefix("duration_ms ")?
        .parse()
        .ok()?;
    let tree = read_line(&data, &mut pos)?.strip_prefix("tree ")?.to_string();
    let cwd = String::from_utf8(read_blob(&data, &mut pos, "cwd")?).ok()?;
    let cmd = String::from_utf8(read_blob(&data, &mut pos, "cmd")?).ok()?;
    let stdout = read_blob(&data, &mut pos, "stdout")?;
    let stderr = read_blob(&data, &mut pos, "stderr")?;
    Some(Entry {
        cwd,
        cmd,
        tree,
        exit_code,
        created,
        duration_ms,
        stdout,
        stderr,
    })
}

pub fn list() -> Vec<Entry> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(results_dir()) {
        for e in rd.filter_map(|e| e.ok()) {
            if let Some(entry) = read_entry(&e.path()) {
                out.push(entry);
            }
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.created));
    out
}

pub fn negative_add(cwd: &str, cmd: &str) -> io::Result<()> {
    let dir = negative_dir();
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join(key(cwd, cmd)),
        format!("{}  [{}]", cmd, cwd),
    )
}

pub fn negative_has(cwd: &str, cmd: &str) -> bool {
    negative_dir().join(key(cwd, cmd)).exists()
}

pub fn negative_list() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(negative_dir()) {
        for e in rd.filter_map(|e| e.ok()) {
            if let Ok(s) = fs::read_to_string(e.path()) {
                out.push(s);
            }
        }
    }
    out.sort();
    out
}

pub struct Lock {
    path: PathBuf,
}

impl Lock {
    pub fn acquire() -> Option<Lock> {
        let _ = fs::create_dir_all(root());
        let path = root().join("speculate.lock");
        if let Some(l) = Self::try_create(&path) {
            return Some(l);
        }
        let stale = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|e| e.as_secs() > 600)
            .unwrap_or(true);
        if stale {
            let _ = fs::remove_file(&path);
            return Self::try_create(&path);
        }
        None
    }

    fn try_create(path: &Path) -> Option<Lock> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .ok()?;
        let _ = write!(f, "{}", std::process::id());
        Some(Lock {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn join_args(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty()
                && !a
                    .chars()
                    .any(|c| c.is_whitespace() || "'\"\\$`".contains(c))
            {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_freshness() {
        let tmp = std::env::temp_dir().join(format!("specsh-cache-{}", std::process::id()));
        unsafe { std::env::set_var("XDG_CACHE_HOME", &tmp) };
        let e = Entry {
            cwd: "/some/dir".to_string(),
            cmd: "cargo test".to_string(),
            tree: "abc123".to_string(),
            exit_code: 3,
            created: now(),
            duration_ms: 4200,
            stdout: b"out bytes\nwith\nnewlines".to_vec(),
            stderr: b"".to_vec(),
        };
        store(&e).unwrap();
        let got = load("/some/dir", "cargo test").unwrap();
        assert_eq!(got.exit_code, 3);
        assert_eq!(got.stdout, e.stdout);
        assert!(got.fresh("abc123"));
        assert!(!got.fresh("zzz"));
        assert!(load("/some/dir", "cargo build").is_none());

        assert!(!negative_has("/some/dir", "curl x"));
        negative_add("/some/dir", "curl x").unwrap();
        assert!(negative_has("/some/dir", "curl x"));
        assert!(!negative_has("/other", "curl x"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn join_quotes_specials() {
        let v: Vec<String> = vec!["cargo".into(), "test".into(), "a b".into()];
        assert_eq!(join_args(&v), "cargo test 'a b'");
    }
}

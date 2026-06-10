use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::UNIX_EPOCH;

const BASIS: u128 = 0x6C62272E07BB014262B821756295C58D;
const PRIME: u128 = (1 << 88) + (1 << 8) + 0x3B;
const MAX_FILES: usize = 100_000;

const PRUNE: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    ".next",
    ".cache",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".turbo",
];

pub fn fnv128(data: &[u8]) -> u128 {
    let mut h = BASIS;
    feed(&mut h, data);
    h
}

pub fn hex128(h: u128) -> String {
    format!("{:032x}", h)
}

fn feed(h: &mut u128, data: &[u8]) {
    for b in data {
        *h ^= *b as u128;
        *h = h.wrapping_mul(PRIME);
    }
}

pub fn tree_hash(root: &Path) -> io::Result<String> {
    let mut h = BASIS;
    let mut count = 0usize;
    visit(root, root, &mut h, &mut count);
    feed(&mut h, &(count as u64).to_le_bytes());
    Ok(hex128(h))
}

fn visit(root: &Path, dir: &Path, h: &mut u128, count: &mut usize) {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if *count > MAX_FILES {
            return;
        }
        let name = e.file_name();
        let ft = match e.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            if PRUNE.contains(&name.to_string_lossy().as_ref()) {
                continue;
            }
            visit(root, &e.path(), h, count);
        } else if ft.is_file() {
            *count += 1;
            let md = match e.metadata() {
                Ok(md) => md,
                Err(_) => continue,
            };
            let path = e.path();
            let rel = path.strip_prefix(root).unwrap_or(&path);
            feed(h, rel.as_os_str().as_bytes());
            feed(h, &md.len().to_le_bytes());
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            feed(h, &mtime.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_then_changes_on_touch() {
        let d = std::env::temp_dir().join(format!("specsh-hash-{}", std::process::id()));
        fs::create_dir_all(d.join("src")).unwrap();
        fs::create_dir_all(d.join("target")).unwrap();
        fs::write(d.join("src/a.rs"), "one").unwrap();
        fs::write(d.join("target/junk"), "x").unwrap();
        let h1 = tree_hash(&d).unwrap();
        assert_eq!(h1, tree_hash(&d).unwrap());
        fs::write(d.join("target/junk"), "different").unwrap();
        assert_eq!(h1, tree_hash(&d).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(d.join("src/a.rs"), "two!").unwrap();
        assert_ne!(h1, tree_hash(&d).unwrap());
        fs::remove_dir_all(&d).unwrap();
    }
}

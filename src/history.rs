use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn default_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/fish/fish_history")
}

pub fn parse(path: &Path) -> io::Result<Vec<String>> {
    let data = fs::read_to_string(path)?;
    if data.starts_with("- cmd:") || data.contains("\n- cmd:") {
        Ok(parse_fish(&data))
    } else {
        Ok(parse_plain(&data))
    }
}

fn parse_fish(data: &str) -> Vec<String> {
    data.lines()
        .filter_map(|l| l.strip_prefix("- cmd: "))
        .map(unescape)
        .collect()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_plain(data: &str) -> Vec<String> {
    data.lines()
        .filter_map(|l| {
            let l = l.trim_end();
            if l.is_empty() {
                return None;
            }
            if let Some(rest) = l.strip_prefix(": ") {
                return rest.split_once(';').map(|x| x.1).map(str::to_string);
            }
            Some(l.to_string())
        })
        .collect()
}

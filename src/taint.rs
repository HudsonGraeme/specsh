use std::process::Command;

pub fn local_timestamp() -> Option<String> {
    let out = Command::new("/bin/date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn sandbox_denials_since(start: &str) -> Vec<String> {
    let out = match Command::new("/usr/bin/log")
        .args([
            "show",
            "--style",
            "compact",
            "--start",
            start,
            "--predicate",
            "sender == \"Sandbox\" AND eventMessage CONTAINS \"deny\"",
        ])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains("deny"))
        .map(str::to_string)
        .collect()
}

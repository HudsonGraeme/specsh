use std::path::Path;

pub const WRAPPED_HEADS: &[&str] = &[
    "cargo", "pytest", "go", "tsc", "eslint", "ruff", "mypy", "pyright", "shellcheck", "npm",
    "pnpm", "yarn", "bun", "forge", "nargo", "uv",
];

const DENY_TOKENS: &[&str] = &["--fix", "--write", "-w", "--watch", "--fork-url"];

pub fn eligible(cmd: &str) -> Result<(), String> {
    if cmd.contains('\n') {
        return Err("multiline".to_string());
    }
    if let Some(c) = cmd.chars().find(|c| "|;&<>`".contains(*c)) {
        return Err(format!("shell operator '{}'", c));
    }
    if cmd.contains("$(") {
        return Err("command substitution".to_string());
    }
    let toks: Vec<&str> = cmd.split_whitespace().collect();
    if toks.is_empty() {
        return Err("empty".to_string());
    }
    for t in &toks {
        if DENY_TOKENS.contains(t) || t.starts_with("--fork-url") {
            return Err(format!("denied flag {}", t));
        }
    }
    let head = basename(toks[0]);
    let second = toks.get(1).copied().unwrap_or("");
    let third = toks.get(2).copied().unwrap_or("");
    let ok = match head {
        "cargo" => matches!(second, "test" | "check" | "clippy" | "build"),
        "pytest" | "eslint" | "mypy" | "pyright" | "shellcheck" => true,
        "python" | "python3" => second == "-m" && third == "pytest",
        "uv" => second == "run" && third == "pytest",
        "go" => matches!(second, "test" | "vet" | "build"),
        "tsc" => toks.contains(&"--noEmit"),
        "ruff" => second == "check",
        "npm" | "pnpm" | "yarn" | "bun" => {
            second == "test"
                || (second == "run" && matches!(third, "test" | "lint" | "typecheck" | "check"))
        }
        "forge" => matches!(second, "test" | "build"),
        "nargo" => second == "test",
        _ => return Err(format!("head '{}' not in allowlist", head)),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("'{} {}' not an output-valued form", head, second))
    }
}

pub fn basename(tok: &str) -> &str {
    tok.rsplit('/').next().unwrap_or(tok)
}

pub fn marker_present(cwd: &Path, head_tok: &str) -> bool {
    let markers: &[&str] = match basename(head_tok) {
        "cargo" => &["Cargo.toml"],
        "go" => &["go.mod"],
        "forge" => &["foundry.toml"],
        "nargo" => &["Nargo.toml"],
        "npm" | "pnpm" | "yarn" | "bun" | "tsc" | "eslint" => &["package.json"],
        "pytest" | "python" | "python3" | "uv" | "mypy" | "pyright" | "ruff" => &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "pytest.ini",
            "requirements.txt",
        ],
        _ => return true,
    };
    markers.iter().any(|m| cwd.join(m).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        assert!(eligible("cargo test").is_ok());
        assert!(eligible("cargo test --release").is_ok());
        assert!(eligible("cargo clippy").is_ok());
        assert!(eligible("pytest -x tests/").is_ok());
        assert!(eligible("pnpm run typecheck").is_ok());
        assert!(eligible("forge test").is_ok());
        assert!(eligible("tsc --noEmit").is_ok());
        assert!(eligible("nargo test").is_ok());
    }

    #[test]
    fn default_deny() {
        assert!(eligible("cargo publish").is_err());
        assert!(eligible("cargo fmt").is_err());
        assert!(eligible("git push origin main").is_err());
        assert!(eligible("curl https://x").is_err());
        assert!(eligible("rm -rf /").is_err());
        assert!(eligible("tsc").is_err());
        assert!(eligible("pnpm run deploy").is_err());
        assert!(eligible("eslint --fix src").is_err());
        assert!(eligible("forge test --fork-url http://x").is_err());
        assert!(eligible("cargo test && git push").is_err());
        assert!(eligible("cargo test > out.txt").is_err());
        assert!(eligible("pytest $(cat list)").is_err());
    }
}

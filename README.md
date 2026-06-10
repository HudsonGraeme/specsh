# specsh

Speculative execution for your shell.

CPUs don't wait to find out which way a branch goes. They predict, execute ahead, and keep the result only if the prediction was right. specsh applies the same idea to the terminal: run the command you are about to type before you type it, in a sandbox that cannot touch anything real, so the output is already there when you ask for it.

This repository currently contains the first milestone: the execution engine. It is useful on its own as a way to run any command against a disposable copy of your working tree with no network and no side effects.

```
$ specsh run -- cargo test
...
specsh: exit 0 in 4.12s (clone discarded)
```

## How it works

```
cwd ──clonefile──▶ throwaway clone ──sandbox-exec──▶ command
                                                        │
                       stdout / stderr / exit code ◀────┘
                       (the clone is deleted; only output survives)
```

1. The working directory is cloned with APFS copy-on-write (`clonefile`), which takes milliseconds and no meaningful disk space. Non-APFS volumes fall back to a plain copy.
2. The command runs inside the clone under a `sandbox-exec` profile that denies all network access and any filesystem write outside the clone.
3. stdout, stderr, and the exit code are captured. The clone is deleted. Nothing is ever merged back.

This is output-only speculation, closer to runahead execution than to ordinary branch prediction: a real CPU commits speculated stores at retirement, specsh never commits anything. The only artifact of a run is the captured output.

## Why it is safe

Safety does not come from the sandbox alone. It comes from the fact that every mechanism of irreversibility is absent inside it.

| Effect class | Containment |
|---|---|
| Filesystem writes | Land in the copy-on-write clone, deleted after capture |
| Network, deploys, publishes | Denied by the sandbox profile, no packets leave the machine |
| Writes outside the tree (dotfiles, caches, sockets) | Denied by the profile |
| Runaway processes | Wall-clock timeout, killed and discarded |

A command capable of an irreversible action either fails inside the sandbox or is never speculated at all. The worst possible outcome of a wrong prediction is wasted CPU and a deleted directory.

## Taint, or why this never lies to you

A test suite that exits 1 because a test genuinely fails is a valid result. A test suite that exits 1 because the sandbox refused a connection is not a result at all, and presenting it as one would be the worst bug this tool could have.

specsh distinguishes the two by evidence, not by guessing from exit codes. After a failing run it checks the unified log for Sandbox denial events from the kernel. If any occurred, the result is marked tainted and reported as such:

```
$ specsh run -- 'curl -sS https://crates.io'
curl: (6) Could not resolve host: crates.io
specsh: exit 6 in 0.03s (clone discarded)
specsh: TAINTED, 1 sandbox denial(s) during run; this failure may be sandbox-induced, not real:
  kernel (Sandbox) curl(98677) deny(1) network-outbound ...
```

Tainted results exit with code 113 and are never to be treated as the command's real outcome. In the full speculation pipeline they go into a negative cache: the command is simply not speculable, and it runs live, untouched, like it always did. Speculation can make commands faster. It can never make them slower or wronger.

## Usage

```
specsh exec -- <command>      serve a speculated result, or exec the live command
specsh speculate [--after CMD] [--only CMD] [--max N] [--timeout SECS]
specsh predict [--after CMD]  show predictions and their eligibility verdicts
specsh run [--timeout SECS] [--keep] -- <command>
specsh status                 show cached results and the negative cache
specsh init                   print the fish integration
specsh profile                print the sandbox profile for the cwd
```

Exit code is passed through from the command. Exit 113 from `run` means the result was tainted by a sandbox denial.

## The full pipeline

Everything from the original roadmap is implemented. The pieces:

**Result cache.** Speculated results are stored under `~/.cache/specsh`, keyed on the working directory, the command, and a hash of the tree state (every file's path, size, and mtime, with build directories like `target` and `node_modules` excluded). A result is served only if the tree hash still matches at serve time, so what you see is provably computed from the files you are looking at. Entries also expire after 15 minutes.

**Negative cache.** A speculation that fails with a Sandbox denial event in the unified log is recorded as non-speculable for that command in that project. It will never be speculated again there; it just runs live, like it always did. The negative cache is per project: `npm test` needing the network in one repo says nothing about another.

**Eligibility, default deny.** Only output-valued command forms are ever speculated: `cargo test|check|clippy|build`, `pytest`, `go test|vet|build`, `tsc --noEmit`, `eslint`, `ruff check`, `mypy`, `pyright`, `shellcheck`, `npm|pnpm|yarn|bun test|lint|typecheck|check`, `forge test|build`, `nargo test`, `uv run pytest`. Anything with shell operators, command substitution, or flags like `--fix`, `--write`, `--watch`, `--fork-url` is refused. A command is also skipped unless the project marker for its toolchain (`Cargo.toml`, `package.json`, `go.mod`, `foundry.toml`, ...) exists in the cwd.

**Predictor.** A pattern history over your shell stream, exactly like a branch predictor's: parse fish history, count transitions between consecutive commands, and rank what follows the command you just ran (with a frequency fallback for unseen contexts). `specsh predict --after 'vim src/main.rs'` shows you the table.

**Speculation runs politely.** Niced to the lowest priority, one speculation cycle at a time under a lock, at most two runs per cycle, with color forced so the cached output looks the way your terminal expects.

## Fish integration

```
specsh init | source
```

Put that in `config.fish` and the loop closes:

1. After every command, a hook fires `specsh speculate --after <that command>` in the background, which predicts what you will run next and warms the cache.
2. Wrapper functions for the eligible heads (`cargo`, `pytest`, `go`, ...) route through `specsh exec`, which serves a fresh cached result instantly or execs the real binary with zero ceremony.

```
$ vim src/lib.rs        (in the background: specsh predicts and runs cargo test)
$ cargo test
...
specsh: served speculated result from 9s ago, saved ~4.1s
```

A wrong prediction costs nothing you will notice: the run was niced, sandboxed, and discarded. An unpredicted command just runs live.

## Install

Requires macOS (APFS `clonefile` and `sandbox-exec`). Zero dependencies, builds with stock cargo:

```
git clone https://github.com/HudsonGraeme/specsh
cd specsh
cargo install --path .
specsh init | source
```

## Known limitations

- Output only: a served `cargo test` does not warm your real `target/` directory, so a later live build may recompile. Artifact promotion is deliberately out of scope for now.
- Taint detection reads Sandbox denial events from the unified log in the run's time window. Unrelated denials from other processes can false-positive, which errs in the safe direction: the command is marked non-speculable and simply runs live.
- The eligibility allowlist is intentionally small. Growing it is a one-line change per toolchain; being wrong in the other direction is not.

## Sibling project

histmine mines your shell history for repeated command templates and synthesizes the fish functions you have been writing by hand. specsh decides what you will run next; histmine figures out what you have been running all along.

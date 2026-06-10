# specsh

Speculative execution for your shell.

CPUs don't wait to find out which way a branch goes. They predict, execute ahead, and keep the result only if the prediction was right. specsh does the same for the terminal: it predicts the command you are about to run, executes it in a sandbox that cannot touch anything real, and serves the output instantly when you ask for it.

```
$ vim src/lib.rs        (specsh predicts and runs cargo test in the background)
$ cargo test
...
specsh: served speculated result from 9s ago, saved ~4.1s
```

## How it works

```
cwd ──clone──▶ throwaway copy ──sandbox──▶ command ──▶ result cache
                                                          │
$ cargo test ──▶ tree unchanged? ──▶ serve instantly ◀────┘
                       └──▶ changed: exec the real thing
```

After every command, a hook predicts what you will run next from the transition history of your shell stream. Eligible predictions run against a copy-on-write clone of the working directory (APFS `clonefile` on macOS, `cp --reflink` on Linux) inside a sandbox with no network and no write access outside the clone (`sandbox-exec` on macOS, bubblewrap on Linux). Output, exit code, and a hash of the tree state go into the cache; the clone is deleted. Nothing is ever merged back.

When you run the command, it is served from cache only if the tree hash still matches and the entry is under 15 minutes old. Otherwise it execs the real binary, untouched.

## Why it is safe

Every mechanism of irreversibility is absent inside the sandbox:

| Effect class | Containment |
|---|---|
| Filesystem writes | Land in the clone, deleted after capture |
| Network, deploys, publishes | Denied by the sandbox |
| Writes outside the tree | Denied by the sandbox |
| Runaway processes | Killed at the timeout, niced to lowest priority |

Speculation is also default deny: only output-valued forms ever run (`cargo test|check|clippy|build`, `pytest`, `go test|vet|build`, `tsc --noEmit`, `eslint`, `ruff check`, `mypy`, `pyright`, `shellcheck`, `npm|pnpm|yarn|bun test|lint|typecheck|check`, `forge test|build`, `nargo test`, `uv run pytest`). Shell operators, command substitution, `--fix`, `--write`, `--watch`, and `--fork-url` are refused, and a command is skipped unless its toolchain's project marker (`Cargo.toml`, `package.json`, `go.mod`, ...) exists in the cwd.

A failure caused by the sandbox is never presented as a real result. On macOS, failing runs are checked against Sandbox denial events in the unified log; a tainted command goes into a per-project negative cache and simply runs live from then on. The worst outcome of a wrong prediction is wasted CPU and a deleted directory.

## Usage

```
specsh exec -- <command>      serve a speculated result, or exec the live command
specsh speculate [--after CMD] [--only CMD] [--max N] [--timeout SECS]
specsh predict [--after CMD]  show predictions and their eligibility verdicts
specsh run [--timeout SECS] [--keep] -- <command>
specsh status                 show cached results and the negative cache
specsh stats                  speculation cost vs payoff
specsh init                   print the fish integration
specsh profile                print the sandbox configuration for the cwd
```

`specsh stats` answers whether speculation is earning its keep: background CPU and peak memory spent on speculative runs (measured via `getrusage`), foreground waiting avoided by served results, hit rate, and the net, per command. The ledger is a local file under `~/.cache/specsh`; nothing leaves the machine.

`specsh run` is the engine standalone: run anything against a disposable clone with no network and no side effects. Exit 113 means the failure was sandbox-induced.

Close the loop in `config.fish`:

```
specsh init | source
```

This installs a `fish_postexec` hook that speculates in the background and wrapper functions that route eligible commands through `specsh exec`.

## Install

From a release (each archive carries build provenance and a SHA256SUMS entry):

```
gh release download -R HudsonGraeme/specsh -p 'specsh-aarch64-apple-darwin.tar.gz' -p 'SHA256SUMS'
shasum -a 256 -c SHA256SUMS --ignore-missing
gh attestation verify specsh-aarch64-apple-darwin.tar.gz -R HudsonGraeme/specsh
tar -xzf specsh-aarch64-apple-darwin.tar.gz
```

Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`. Linux requires bubblewrap (`apt install bubblewrap`).

Or from source, zero dependencies:

```
git clone https://github.com/HudsonGraeme/specsh
cd specsh && cargo install --path .
```

## License

MIT.

## Limitations

- Output only: a served `cargo test` does not warm your real `target/`, so a later live build may recompile. Artifact promotion is deliberately out of scope.
- Taint detection on macOS reads the unified log in the run's time window; unrelated denials can false-positive, which errs in the safe direction (the command just runs live). Linux relies on output heuristics.
- The eligibility allowlist is intentionally small. Growing it is a one-line change per toolchain; being wrong in the other direction is not.

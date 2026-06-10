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
specsh run [--timeout SECS] [--keep] -- <command>
specsh profile
```

| Flag | Meaning |
|---|---|
| `--timeout SECS` | Kill the command after this many seconds (default 300) |
| `--keep` | Keep the clone on disk for inspection instead of deleting it |
| `profile` | Print the generated sandbox profile for the current directory |

Exit code is passed through from the command. Exit 113 means the result was tainted by a sandbox denial.

## Install

Requires macOS (APFS `clonefile` and `sandbox-exec`). Zero dependencies, builds with stock cargo:

```
git clone https://github.com/HudsonGraeme/specsh
cd specsh
cargo build --release
```

The binary is `target/release/specsh`.

## Roadmap

The execution engine is layer one. The remaining pieces of the speculation pipeline, in order:

- Result cache keyed on a hash of the tree state, so a served result is provably computed from the files you are looking at
- Negative cache for commands proven non-speculable by taint
- Eligibility classification: only output-valued commands (tests, lints, typechecks, builds) are ever speculated, default deny
- The predictor: pattern history over your shell stream, deciding what to run ahead

## Sibling project

histmine mines your shell history for repeated command templates and synthesizes the fish functions you have been writing by hand. specsh decides what you will run next; histmine figures out what you have been running all along.

# hush

Trim the noise out of dev-command output for LLM coding agents, and never transmit it.

`hush` runs a command and prints a compact version of its output: build and test logs
shrink to failures and totals, progress redraws and color codes disappear. Whatever it
changes is stored locally and comes back byte for byte with `hush expand <id>`.

The process that does this cannot reach the network. Before it reads any output, hush
closes an irreversible, kernel-enforced network gate on itself (macOS `sandbox_init`,
Linux `seccomp`). `hush doctor` proves it on your machine.

```
$ hush git diff
6 files changed (+553 -2):
.gitignore  (+2 -0)
Cargo.lock  (+480 -0)
...
[hush:git-diff id=8a3f783e0830 lines=608->7 - `hush expand 8a3f783e0830` for full output]
```

## Table of Contents

- [Principles](#principles)
- [Two ways to use it](#two-ways-to-use-it)
- [Compression](#compression)
- [Install](#install)
- [Usage](#usage)
  - [Claude Code integration](#claude-code-integration)
- [How it works](#how-it-works)
- [Platform support](#platform-support)
- [Development](#development)
- [License](#license)

## Principles

The model decides what to read. Current coding agents already narrow output themselves:
they pipe into `head`, `grep`, or `jq`, read line ranges with `sed -n`, and ask for
`git diff --stat` when a stat is enough. hush does not second-guess that. It never cuts
output the model shaped, and it never cuts content the model asked to read.

What hush removes is noise no reader wants: ANSI color codes, `\r` progress-bar redraws,
long runs of the same line, and the hundreds of passing-test and `Compiling foo` lines
around the one failure that matters.

Fold, don't drop. Every change is reversible. The original is content-addressed under
`~/.local/share/hush/`, and `hush expand <id>` returns the exact bytes, so the model never
re-runs a command to recover context.

Nothing leaves the machine. Filtering, storing, and expanding all happen behind the
network gate. hush has no telemetry and no external URLs.

## Two ways to use it

Wrap a command when you want a summary. `hush <command>` picks a filter for the command
and folds aggressively, because you asked for it:

- version control: `git status/diff/log/show`, `diff`
- builds and linters: `cargo build/clippy/check`, `go build/vet`, `tsc`, `eslint`, `make`,
  `docker build`
- test runners: `cargo test`, `go test`, `pytest`, `jest`, `vitest`, `mocha`, ...
- package installs: `npm`/`pnpm`/`yarn`/`bun install`, `pip install`
- tables: `docker ps`, `kubectl get`, `ps`, `df`, `pip list`, `lsblk`, `free`, `ss`
- the filesystem and search: `ls`, `find`, `du`, `tree`, `grep`
- JSON: any output requested with `-o json`, `--format json`, `--json`, ... or that sniffs
  as JSON/NDJSON gets large arrays summarized and long strings clipped
- `hush read <file> --signatures`: just the signatures, via tree-sitter
- anything else: blank and duplicate lines collapsed, head and tail kept

Install the Claude Code hook when you want noise removed from every Bash result without
thinking about it. The hook is deliberately conservative:

| Bash command the model ran | What the hook does |
|---|---|
| `hush ...`, anything piped into `head`/`tail`/`grep`/`jq`/..., `cat`/`sed`/`git`/`gh`, anything behind `env`/`sh -c`/`xargs`/... | nothing |
| a plain build, test, or install (`cargo test`, `cd app && npm install`, ...), but not list/dry-run/help modes such as `cargo test -- --list` or `make -n` | summarize with the command's filter |
| anything else | strip ANSI codes, resolve `\r` redraws, fold runs of 3+ identical lines |

The hook replaces a result only when the compacted body is at least 256 bytes smaller than
the original (before the roughly 110-byte expand footer is added).
Claude Code already spills very large output to a file and shows a preview of its head;
for build logs the failure is at the tail, which is why those get a real summary.

## Compression

Measured compaction ratio per command for the wrap path (`hush <command>`) over fixed
sample inputs (`tests/fixtures/`); bytes are raw stdout+stderr vs the compacted body (the
`expand` footer is excluded). Regenerated from the fixtures and refreshed automatically by
CI after each merge to `main`. The hook path is narrower by design and is not measured here.

<!-- compression-report:start -->
```
              hush compression report
---------------------------------------------------
                30 sample commands
---------------------------------------------------
  original      214 KB   4,122 lines   ~53.5K tok
  compressed   47.6 KB     632 lines   ~11.9K tok
  saved         166 KB       (77.8%)   ~41.6K tok
---------------------------------------------------
  by command
  ls                        57.2 KB -> 1.8 KB   97%
  json (cargo messages)     27.8 KB -> 8.0 KB   71%
  json (kubectl -o json)    24.2 KB -> 6.9 KB   72%
  git log                   12.8 KB -> 1.5 KB   88%
  grep                       9.8 KB ->  868 B   91%
  build log (passthrough)    9.7 KB -> 2.2 KB   78%
  du -a                      7.0 KB ->  760 B   89%
  git show                   4.4 KB ->  735 B   83%
  npm install                4.6 KB -> 1.0 KB   77%
  make                       3.6 KB ->  635 B   83%
  kubectl logs               4.4 KB -> 1.5 KB   66%
  pytest                     7.0 KB -> 4.1 KB   41%
  diff                       2.8 KB ->  169 B   94%
  go test                    2.8 KB ->  294 B   89%
  docker build               3.1 KB ->  742 B   76%
  docker ps                  6.5 KB -> 4.1 KB   37%
  tree                       3.0 KB ->  675 B   78%
  cargo test                 2.5 KB ->  331 B   87%
  git diff                   2.0 KB ->   94 B   95%
  vitest                     2.3 KB ->  573 B   75%
  docker build (buildkit)    2.1 KB ->  700 B   66%
  python (traceback)         2.7 KB -> 1.4 KB   50%
  pip list                   1.5 KB ->  735 B   52%
  eslint                     2.4 KB -> 1.8 KB   25%
  find                        779 B ->  258 B   67%
  tsc                        3.2 KB -> 2.9 KB   10%
  go build                   2.2 KB -> 1.9 KB   14%
  git status                  883 B ->  583 B   34%
  cargo build                 390 B ->  183 B   53%
  cargo build (cargo err)     225 B ->  200 B   11%
---------------------------------------------------
     ~tok = bytes/4, from fixed sample inputs
```
<!-- compression-report:end -->

## Install

Homebrew (prebuilt binary, no compile):

```sh
brew tap Matuyuhi/tools
brew install --formula hush          # = Matuyuhi/tools/hush
```

From source (requires Rust):

```sh
cargo install --path .
# or: cargo build --release  (binary at target/release/hush)
```

## Usage

```sh
hush cargo test                 # failures and totals
hush git diff                   # per-file +N/-M summary (hunks via expand)
hush git log                    # one line per commit
hush grep -rn TODO src/         # hits grouped per file
hush find . -name '*.rs'        # paths grouped by directory
hush read src/main.rs --signatures   # AST signatures only (tree-sitter)

hush expand <id>                # print the full original output
hush stats                      # how much has been compressed so far
hush doctor                     # prove the non-transmission sandbox works
```

Any command works; the wrapped command's exit code is propagated unchanged. When nothing
is changed, hush prints the output as is and creates no expand id.

### Claude Code integration

```sh
hush install                    # project scope (.claude/)
hush install --user             # user scope (~/.claude/)
hush uninstall [--user]         # remove it again
```

This adds a `PostToolUse` hook on Bash (see the table under
[Two ways to use it](#two-ways-to-use-it)) and a short `HUSH.md` guide imported from
`CLAUDE.md`. Any problem inside the hook (unparsable input, the gate failing, a filter
error) makes it a no-op, so your Bash results are never broken.

Codex is not supported. Codex hooks cannot replace a tool's output after it runs, and the
only way to wrap a command beforehand also auto-approves it, skipping Codex's approval
prompt. `hush install` and `hush uninstall` remove the Codex hook that hush 0.4.0 installed.

## How it works

```
hush <command>:
  1. run the command, capture stdout/stderr/exit code   (network is fine here)
  2. sandbox::gate()   <-- irreversible: this process can no longer transmit
  3. filter -> store the original -> print the compact form + an expand footer

hush hook (Claude Code PostToolUse):
  1. read the finished Bash result from stdin
  2. sandbox::gate()
  3. route (skip / summarize / denoise) -> store -> return the replacement
```

- The gate denies all outbound network for the process. macOS uses an `sandbox_init`
  SBPL profile (`deny network*`); Linux uses a `seccomp` filter that refuses
  `socket(AF_INET/AF_INET6/AF_PACKET)`. If the gate can't be established, hush fails
  closed (override only with `HUSH_ALLOW_NO_SANDBOX=1`).
- `hush doctor` probes the network before and after the gate and shows that outbound
  connections are open beforehand and blocked afterward, so the gate itself is what stops
  transmission.
- The store is content-addressed; identical outputs dedupe, and `expand` is byte-exact.
  `hush gc [--days N]` prunes it.

## Platform support

macOS and Linux (both verified in CI, including `hush doctor` on each). Other platforms
build but fail closed: the gate can't be guaranteed, so hush refuses to run the filter.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --no-default-features   # core only; drops tree-sitter (the `ast` feature)
cargo run -- doctor
```

A compression benchmark runs the filters over fixed sample inputs in `tests/fixtures/`
(`cargo test --test compression`): it fails if any command's compaction ratio drops below
a per-command floor, and writes a markdown report of the ratio per command. CI publishes
that report to the job summary on every push to `main`, and posts it as a PR comment when
the `compression-report` label is added.

Releases are cut from `Cargo.toml`'s version: run the Bump version GitHub Action
(patch/minor/major), merge the PR it opens, and `release.yml` builds the per-platform
binaries, publishes a GitHub Release, and updates the Homebrew tap.

## License

Apache-2.0.

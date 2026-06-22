# hush

**Compress dev-command output to cut LLM token usage — and physically never transmit it.**

`hush` wraps a command, runs it, and prints a compacted version of its output so an
LLM coding agent burns far fewer tokens reading it. When compaction elides anything,
the full original is stored locally and recovered on demand with `hush expand <id>`.
If nothing is elided, `hush` prints the full output and does not create an expand id.

The headline property is **non-transmission**. Before `hush` filters or renders any output, it
closes an irreversible, kernel-enforced network gate on its own process (macOS
`sandbox_init`, Linux `seccomp`). The compression/filtering code therefore *cannot* send your
code or command output anywhere — and you can prove it: `hush doctor`.

```
$ hush git diff
6 files changed (+553 -2):
.gitignore  (+2 -0)
Cargo.lock  (+480 -0)
...
[hush:git-diff id=8a3f783e0830 lines=608->7 - `hush expand 8a3f783e0830` for full output]
```

## Table of Contents

- [Why](#why)
- [Compression](#compression)
- [Install](#install)
- [Usage](#usage)
  - [Claude Code integration](#claude-code-integration)
- [How it works](#how-it-works)
- [Platform support](#platform-support)
- [Development](#development)
- [License](#license)

## Why

- **Non-transmission by construction** — a one-way gate is applied *before* any
  filtering runs, so the hot path has no way to reach the network. No telemetry, no
  external URLs, ever.
- **Fold, don't drop** — compaction is reversible. Every elided output is content-
  addressed and stored under `~/.local/share/hush/`; `hush expand <id>` returns the
  exact bytes, so the model never has to re-run a command for missing context.
- **Per-command filters** — a tailored compaction for each family: version control
  (`git status/diff/log/show`, `diff`), builds and linters (`cargo build/test`,
  `go build/vet/test`, `tsc`, `eslint`, `make`), test runners (`jest`/`vitest`/`mocha`/
  `pytest`/…), package installs (`npm`/`pnpm`/`yarn`/`bun`/`pip install`), tables
  (`docker ps`, `kubectl get`, `ps`, `df`, `pip list`, `lsblk`, `free`, …), the
  filesystem (`ls`, `find`, `du`, `tree`), `grep`, Python tracebacks, and `read`
  (tree-sitter signatures). Anything else falls back to a generic compactor.
- **Content-aware JSON** — any command that emits JSON or NDJSON is detected by an
  output flag (`-o json`, `--format json`, `--message-format=json`, `--json`, …) or by
  sniffing the bytes, then large arrays are summarized, long strings clipped, and
  whitespace folded — so `kubectl -o json`, `gh … --json`, `cargo --message-format=json`,
  `cat foo.json`, and friends all compact without per-command wiring.

## Compression

Measured compaction ratio per command over fixed sample inputs (`tests/fixtures/`);
bytes are raw stdout+stderr vs the compacted body (the `expand` footer is excluded).
Regenerated from the fixtures and refreshed automatically by CI after each merge to `main`.

<!-- compression-report:start -->
```
              hush compression report
---------------------------------------------------
                27 sample commands
---------------------------------------------------
  original      204 KB   3,943 lines   ~51.1K tok
  compressed   44.7 KB     579 lines   ~11.2K tok
  saved         160 KB       (78.1%)   ~39.9K tok
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
  pytest                     7.0 KB -> 4.1 KB   41%
  diff                       2.8 KB ->  169 B   94%
  go test                    2.8 KB ->  294 B   89%
  docker ps                  6.5 KB -> 4.1 KB   37%
  tree                       3.0 KB ->  675 B   78%
  cargo test                 2.5 KB ->  331 B   87%
  git diff                   2.0 KB ->   94 B   95%
  vitest                     2.3 KB ->  573 B   75%
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
hush git status                 # compact branch + changed files
hush git diff                   # per-file +N/-M summary (hunks via expand)
hush git log                    # one line per commit
hush grep -rn TODO src/         # hits grouped per file
hush find . -name '*.rs'        # paths grouped by directory
hush cat path/to/file           # generic compaction
hush read src/main.rs --signatures   # AST signatures only (tree-sitter)

hush expand <id>                # print the full original output
hush stats                      # how much has been compressed so far
hush doctor                     # prove the non-transmission sandbox works
```

Any command works — unknown ones use a generic compactor (blank/duplicate collapse,
head + expand). The wrapped command's exit code is propagated unchanged.

### Claude Code integration

```sh
hush install                    # project scope (.claude/); --user for ~/.claude/
```

This adds a `PostToolUse` hook that compresses Bash output before the model sees it,
drops a short `HUSH.md` usage guide, and references it from `CLAUDE.md`. Remove it with
`hush uninstall`.

## How it works

```
hush <command>:
  1. run the command, capture stdout/stderr/exit code   (network is fine here)
  2. sandbox::gate()   <-- irreversible: this process can no longer transmit
  3. filter -> store the original -> print the compact form + an expand footer
```

- **The gate** denies all outbound network for the process. macOS uses an
  `sandbox_init` SBPL profile (`deny network*`); Linux uses a `seccomp` filter that
  refuses `socket(AF_INET/AF_INET6/AF_PACKET)`. If the gate can't be established, hush
  fails closed (override only with `HUSH_ALLOW_NO_SANDBOX=1`).
- **`hush doctor`** probes the network before and after the gate and shows that
  outbound connections are open beforehand and blocked afterward — so the gate itself
  is what stops transmission.
- **The store** is content-addressed; identical outputs dedupe, and `expand` is byte-
  exact. `hush gc [--days N]` prunes it.

## Platform support

macOS and Linux (both verified in CI, including `hush doctor` on each). Other
platforms build but fail closed — the gate can't be guaranteed, so hush refuses to run
the filter.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --no-default-features   # core only; drops tree-sitter (the `ast` feature)
cargo run -- doctor
```

A compression benchmark runs the filters over fixed sample inputs in
`tests/fixtures/` (`cargo test --test compression`): it fails if any command's
compaction ratio drops below a per-command floor, and writes a markdown report of
the ratio per command. CI publishes that report to the job summary on every push
to `main`, and posts it as a PR comment when the `compression-report` label is added.

Releases are cut from `Cargo.toml`'s version: run the **Bump version** GitHub Action
(patch/minor/major), merge the PR it opens, and `release.yml` builds the per-platform
binaries, publishes a GitHub Release, and updates the Homebrew tap.

## License

Apache-2.0.

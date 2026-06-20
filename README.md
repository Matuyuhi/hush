# hush

**Compress dev-command output to cut LLM token usage — and physically never transmit it.**

`hush` wraps a command, runs it, and prints a compacted version of its output so an
LLM coding agent burns far fewer tokens reading it. When compaction elides anything,
the full original is stored locally and recovered on demand with `hush expand <id>`.
If nothing is elided, `hush` prints the full output and does not create an expand id.

The headline property is **non-transmission**. Before `hush` touches any output, it
closes an irreversible, kernel-enforced network gate on its own process (macOS
`sandbox_init`, Linux `seccomp`). The compression code therefore *cannot* send your
code or command output anywhere — and you can prove it: `hush doctor`.

```
$ hush git diff
6 files changed (+553 -2):
.gitignore  (+2 -0)
Cargo.lock  (+480 -0)
...
[hush:git-diff id=8a3f783e0830 lines=608->7 - `hush expand 8a3f783e0830` for full output]
```

## Why

- **Non-transmission by construction** — a one-way gate is applied *before* any
  filtering runs, so the hot path has no way to reach the network. No telemetry, no
  external URLs, ever.
- **Fold, don't drop** — compaction is reversible. Every elided output is content-
  addressed and stored under `~/.local/share/hush/`; `hush expand <id>` returns the
  exact bytes, so the model never has to re-run a command for missing context.
- **Per-command filters** — `git status/diff/log`, `grep`, `find`, `ls`, `cat`,
  `cargo test`, and `read` (tree-sitter signatures) each get a tailored compaction;
  anything else falls back to a generic one.

## Install

Homebrew (prebuilt binary, no compile):

```sh
brew tap Matuyuhi/tools
brew install hush          # = Matuyuhi/tools/hush
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

Releases are cut from `Cargo.toml`'s version: run the **Bump version** GitHub Action
(patch/minor/major), merge the PR it opens, and `release.yml` builds the per-platform
binaries, publishes a GitHub Release, and updates the Homebrew tap.

## License

Apache-2.0.

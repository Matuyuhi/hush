# CLAUDE.md — working on the hush codebase

`hush` wraps a dev command, compresses its output to save LLM tokens, and is
architecturally unable to transmit anything. See `README.md` for the user-facing
overview; this file is guidance for editing the code.

## Layout

```
src/
  main.rs            entry + dispatch
  cli.rs             clap definitions
  error.rs           Error type (no anyhow/thiserror; std only)
  paths.rs           XDG data dir (~/.local/share/hush)
  ui.rs              framed terminal output: Row::{Rule,Line,Center}, render(), center(), commas()
  sandbox/           the non-transmission gate
    mod.rs           deny_network() dispatch + fail-closed gate()
    macos.rs         sandbox_init FFI (SBPL "deny network*")
    linux.rs         seccomp: refuse socket(AF_INET/AF_INET6/AF_PACKET)  [x86_64/aarch64 only]
    unsupported.rs   fail-closed fallback
  exec/              runner (spawn + capture) + pipeline (enforces the order below)
  filters/           per-command compaction; pure (bytes in -> FilterOutput); registry in mod.rs
  store/             content-addressed expand artifacts (+ id.rs)
  ast/               tree-sitter signatures (feature = "ast", default on)
  commands/          doctor / expand / gc / read / stats / install / hook handlers
```

## Invariants — do not break these

- **Non-transmission ordering.** `exec/pipeline.rs` is the only place that runs a wrapped
  command, and it must do: run child → `sandbox::gate()` → filter/store/print. The gate is
  irreversible. Never add code that touches the network on the filter/expand path, and never
  reorder so filtering happens before the gate. Subcommands that don't spawn a child (expand,
  gc, stats, read) call `sandbox::gate()` at their start.
- **ASCII-only output.** All user-facing output is ASCII. Do **not** use East-Asian
  ambiguous-width characters (`─ · → × ✓ ✗`): they render double-width in CJK terminals and
  break column alignment. Use `-`, `->`, `x`, words. Build framed/tabular output through
  `ui.rs` (widths are computed from the data so layout never breaks).
- **English output.** All user-facing strings (help text, footers, messages, errors,
  `HUSH.md`) are English. Code comments are Japanese (matching the existing style).
- **Filters are pure.** A filter takes `FilterInput { argv, stdout, stderr }` and returns
  `FilterOutput`. Add one under `filters/`, register it in `filters/mod.rs::run`. When
  compaction elides anything, return the full original so `finalize` stores it for `expand`.
- **Lose nothing.** The store is content-addressed and `expand` is byte-exact; `store::get`
  guards against path traversal (ids are alphanumeric).

## Build / test

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings   # CI denies warnings
cargo test
cargo build --no-default-features           # core only (AST off)
cargo run -- doctor                          # must print PASS
```

## CI / release

- `ci.yml` runs on macOS + Linux (pinned runners): fmt, clippy, build, test, core-only build,
  and `hush doctor` as a live per-platform sandbox assertion.
- Releasing is driven by `Cargo.toml`'s version. Run the **Bump version** Action
  (patch/minor/major) → merge the PR it opens → `release.yml` builds the four platform
  binaries, publishes a GitHub Release (draft → upload → publish, for immutable releases),
  and updates the `Matuyuhi/homebrew-tools` tap. Do not hand-cut tags.
- Third-party Actions are limited to official `actions/checkout` and `actions/cache`, pinned
  to commit SHAs. Keep it that way.

## Conventions

- Conventional commit subjects (`feat:`, `fix:`, `ci:`, `refactor:`, `docs:`); end commit
  messages with the `Co-Authored-By` trailer.
- Land changes via PRs; `main` requires them (branch ruleset).

## Note: this repo runs hush on itself

`hush install` is set up here (a PostToolUse hook in the gitignored `.claude/`), so some Bash
output you see may be compacted with a `[hush:... id=… ]` footer. Run `hush expand <id>` to get
the full original. `hush doctor` confirms nothing is transmitted.

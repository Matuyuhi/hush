# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`hush` wraps a dev command, compresses its stdout/stderr to save LLM tokens, and is
architecturally unable to transmit anything. See `README.md` for the user-facing overview;
this file is for editing the code.

## Commands

```sh
cargo build                        # debug build (default features incl. `ast`)
cargo build --no-default-features  # core only — drops tree-sitter (the `ast` feature)
cargo test                         # all tests
cargo test strip_ansi              # a single test (substring match on the test name)
cargo fmt --check                  # CI enforces formatting
cargo clippy --all-targets -- -D warnings   # CI denies warnings
cargo run -- doctor                # prove the non-transmission gate (must print PASS)
cargo run -- git status            # run any command through hush during dev
```

CI (`ci.yml`) enforces, on macOS + Linux: fmt / clippy(-D warnings) / build / test /
`--no-default-features` build / `hush doctor` — keep those green. The remaining examples are
dev conveniences, not CI steps.

## Architecture (the big picture)

The whole point is **non-transmission**, and it lives in one ordering that several files
conspire to keep:

- `exec/pipeline.rs::run_wrapped` is the only path that runs a wrapped command. It must do
  **run child → `sandbox::gate()` → filter → store → print**. The gate is an *irreversible*,
  kernel-enforced network shutoff (macOS `sandbox_init` SBPL `deny network*`; Linux `seccomp`
  refusing `socket(AF_INET/AF_INET6/AF_PACKET)`). The child runs *before* the gate (so e.g.
  `git fetch` works); everything after the gate physically cannot transmit. `sandbox/` selects
  the implementation per OS and is **fail-closed** (`gate()` errors unless `HUSH_ALLOW_NO_SANDBOX=1`).
  Subcommands that don't spawn a child (`expand`/`gc`/`stats`/`read`) call `sandbox::gate()` first.

- **Filters** (`filters/`) are the compaction. `filters/mod.rs::run` dispatches on argv
  (`git status/diff/log`, `cargo build|clippy|check`, `cargo test`, `go test`/`pytest`/`jest`,
  `grep`, `find`, `ls`, `docker ps`/`kubectl get`/`ps`/`df`, …) and falls back to `passthrough`.
  Each filter is a **pure** `FilterInput { argv, stdout, stderr } -> FilterOutput`; it never
  touches the store, network, or process. Shared compaction primitives live in
  `filters/common.rs` (`strip_ansi`, `collapse_blank_runs`, `dedup_all`, `truncate_head_tail`,
  `group_paths_by_dir`, `combine_raw`) — build new filters from these. When a filter elides
  anything it returns the full original in `FilterOutput.original`; `filters::finalize` then
  stores it and appends the footer
  `[hush:<filter> id=<ID> lines=A->B - `hush expand <ID>` for full output]` (see `filters/render.rs`).

- **Lazy expansion** (`store/`): the original output is saved content-addressed under
  `$XDG_DATA_HOME/hush` (`~/.local/share/hush`); `hush expand <id>` returns it byte-exact, so
  the model never re-runs a command for lost context. `store::get` is path-traversal guarded.

- **AST signatures** (`ast/`, `feature = "ast"`): `hush read --signatures` extracts just the
  signatures via tree-sitter. `ast::signatures` picks a language by file extension and drives a
  single generic walker with a per-language `Spec` (definition kinds, container kinds to recurse
  into, transparent wrappers). Adding a language = one `spec_for` arm. Currently rs/py/go/
  ts/tsx/js/jsx/mjs/cjs.

- **Framed output** (`ui.rs`): `stats`/`doctor`/`gc`/`install`/`uninstall` render through
  `ui::render(&[Row])` (`Row::{Rule,Line,Center}`), plus `commas` / `human_count` / `human_bytes`.
  Width is computed from the data so the layout never breaks.

- `commands/` holds one handler per subcommand: `doctor` (before/after socket probe proving the
  gate), `expand`, `gc`, `read`, `stats`, `install`/`uninstall` (Claude Code integration), and
  `hook` (the PostToolUse handler — fail-safe: any problem is a no-op so the user's Bash is
  never broken).

`error.rs` is a std-only `Error` enum (no anyhow/thiserror). `paths.rs` resolves the XDG dir.

## Invariants — do not break these

- **The gate ordering above.** Never add network-capable code on the filter/expand path; never
  let filtering run before `sandbox::gate()`.
- **ASCII-only framed output.** `ui.rs` widths assume one column per `char`, so framed/tabular
  output must avoid East-Asian ambiguous-width glyphs (`─ · → × ✓ ✗`) that render double-width
  in CJK terminals and shift columns. Use `-`, `->`, `x`, or words.
- **English user-facing strings** (help, footers, messages, errors, `HUSH.md`). Code comments
  are Japanese, matching the existing style.
- **Filters stay pure and lossless** — see Architecture.

## Release

Releases are driven by `Cargo.toml`'s `version`; **do not hand-cut tags**. Run the
**Bump version** GitHub Action (patch/minor/major) → it opens a version-bump PR → merging it
triggers `release.yml`, which builds the four target binaries, publishes a GitHub Release
(draft → upload → publish, required for GitHub immutable releases), and pushes the formula to
the `Matuyuhi/homebrew-tools` tap. Third-party Actions are limited to official
`actions/checkout` and `actions/cache`, pinned to commit SHAs — keep it that way.

## Conventions

- Conventional commit subjects (`feat:`/`fix:`/`ci:`/`refactor:`/`docs:`); end commit messages
  with the `Co-Authored-By` trailer.
- Land changes via PRs; `main` requires them (branch ruleset, incl. Copilot review).

## Note: this repo runs hush on itself

`hush install` is set up here (a PostToolUse hook in the gitignored `.claude/`), so some Bash
output may be compacted with a `[hush:<filter> id=<ID> ...]` footer — run `hush expand <ID>` for
the full original. `hush doctor` confirms nothing is transmitted.

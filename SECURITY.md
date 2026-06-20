# Security Policy

hush's central design goal is **non-transmission**: the process that compresses your
command output is put behind an irreversible, kernel-enforced network gate *before* any
filtering runs, so it cannot send your code or output anywhere. This document describes
what that guarantees, how to verify it, the known limitations, and how to report issues.

## Supported versions

hush is pre-1.0; only the latest release receives security fixes.

| Version       | Supported |
| ------------- | --------- |
| latest 0.1.x  | yes       |
| anything older | no       |

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue.

- Use GitHub's private vulnerability reporting: the repository's **Security** tab →
  **Report a vulnerability**.
- Include reproduction steps, the affected version (`hush --version`), and the platform
  (macOS/Linux, arch).
- Expect an initial response within a few days. Accepted issues are addressed in a patch
  release; reporters are credited unless they prefer otherwise.

## Security model

### The non-transmission gate

Before hush filters or stores any output, it irreversibly denies outbound network for
its own process:

- **macOS** — `sandbox_init` with an SBPL profile that denies all `network*` operations.
- **Linux** — a `seccomp` filter that refuses `socket(AF_INET / AF_INET6 / AF_PACKET)`.

The wrapped command runs *before* the gate is closed (so e.g. `git fetch` still works);
the gate applies only to hush's own compression/expand code. hush ships no networking
code and contacts no external services — no telemetry, no update checks.

**Fail-closed**: if the gate cannot be established, hush refuses to run the filter. The
only override is the explicit environment variable `HUSH_ALLOW_NO_SANDBOX=1`, which
prints a warning and drops the guarantee. Do not set it unless you understand the
consequence.

### Verify it yourself

`hush doctor` attempts an outbound connection *before* and *after* the gate and shows
that it is possible beforehand and blocked (`EPERM`/`EACCES`) afterward — demonstrating that the
gate, not luck, is what stops transmission. CI runs `hush doctor` on both macOS and Linux
on every change.

### Scope and limitations

- hush sandboxes **its own process**, not the command it wraps. A wrapped command can do
  anything it normally would (including network access); hush only guarantees that *its
  own* filtering/expand step cannot transmit.
- On macOS the gate blocks network *operations* (connect/sendto/bind). Creating a socket
  file descriptor may still succeed, but nothing can be sent. The guaranteed property is
  "no outbound transmission", which `hush doctor` checks directly.
- Name resolution on macOS is performed by a system daemon over IPC, not an in-process
  network socket, so it is outside the scope of the in-process gate (and hush does not use
  it).
- The expand store under `$XDG_DATA_HOME/hush/` (default `~/.local/share/hush/`) holds
  **plaintext** copies of captured command output. Treat it like any local cache — it lives within
  your own file permissions. Prune it with `hush gc`. Avoid wrapping commands whose output must never
  touch local disk.
- Non-Unix platforms have no gate implementation and fail closed (hush refuses to filter).

### Dependencies & supply chain

Dependencies are kept minimal and locked via `Cargo.lock`. GitHub Actions are limited to
the official `actions/checkout` and `actions/cache`, pinned to commit SHAs. Release
binaries are built in CI from tagged commits.

# Contributing to RustPacker

RustPacker is a four-crate Cargo workspace:

| Crate | What it is |
|---|---|
| `rustpacker-core` | Schema, payload format, journal, install log, finish-page logic — platform-independent and unit-tested everywhere |
| `rustpacker-cli` | `init` / `validate` / `pack` / `verify` / `list` / `extract` / `schema` / `version` |
| `rustpacker-stub` | The Win32 installer wizard copied into every `Setup.exe` (Windows only) |
| `rustpacker-gui` | The Builder Studio (Windows only) |

## Building and checking

You need a stable Rust toolchain. The MSRV is **1.70** (`rust-version` in
`Cargo.toml`), so please do not use standard-library APIs newer than that —
CI builds on current stable and will not catch an MSRV regression by itself.

The checks CI runs (`.github/workflows/ci.yml`), and the fastest way to
reproduce them locally:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo deny check` (advisories, licences, bans, sources) runs in the `audit`
job; the licence allow-list lives in `deny.toml`.

The stub and GUI are Windows binaries. On Linux they compile only through
their `cfg(not(windows))` stubs, so anything inside `#[cfg(windows)]` code
is compile-verified by the `windows-msvc` CI job but **not** run: there is
no substitute for testing wizard/GUI behaviour on a real Windows 11 host.
Please say in your PR description what you verified on Windows, or that
you couldn't.

## Conventions

- **Shared logic lives in core.** If the CLI and the Builder Studio both do
  something (file-name sanitising, install-dir validation, the portable
  ZIP), the implementation belongs in `rustpacker-core` with unit tests,
  and both surfaces call it — see AUDIT item M14 for why.
- **Errors are values.** Core returns `Result<_, String>` with user-facing
  messages; the stub's logging never panics (a logging failure must not
  fail an install).
- **Record what you changed.** Notable changes get a `CHANGELOG.md` entry
  under `[Unreleased]`, and fixes for audited issues update the status in
  `AUDIT.md` in the same PR.
- **Licences.** New dependencies must satisfy `deny.toml` (MIT /
  Apache-2.0 / BSD / Unicode-3.0 family). Prefer not adding any: the stub
  is copied into every generated installer, so its dependency tree is kept
  deliberately tiny.

## Cutting a release

1. Bump `version` in `Cargo.toml` and move the `[Unreleased]` changelog
   entries under the new version heading.
2. Tag `v<version>` and push the tag.
3. `.github/workflows/release.yml` builds on Windows and **refuses to
   publish if the tag disagrees with `Cargo.toml`**. A `workflow_dispatch`
   run with `dry_run` exercises the whole path without publishing.

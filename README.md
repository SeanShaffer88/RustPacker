# RustPacker

## TL;DR

RustPacker builds single-file Windows installers: a small native Win32 stub
(`Setup.exe`) with a compressed payload appended to it, plus an optional
portable `.zip` and `SHA256SUMS.txt`. It is 100% offline, has zero
telemetry, targets Windows 11 x64 only, and statically links the C runtime
(`crt-static`) so generated installers need no VC++ redistributable, .NET,
or WebView2 runtime.

## What it does

- **Builder Studio (`rustpacker-gui`)** — Gruvbox-themed studio (light
  cream chrome, deep-green toolbar) with a menu bar, toolbar, grouped
  section navigation, a four-step project flow (Package identity →
  Payload → User experience → Build), an expert-controls strip for the
  advanced pages, and a per-section editor, plus a live preview of the
  installer wizard — in the project's own Gruvbox dark terminal palette —
  including the finish page exactly as the end user will see it.
- **CLI (`rustpacker-cli`)** — every capability from the command line:
  `init`, `validate`, `pack`, `verify`, `list`, `extract`, `schema`, with
  `--json` output and deterministic exit codes (0 ok, 2 usage,
  3 validation, 4 I/O or payload error).
- **Generated installer (`Setup.exe`)** — compact classic wizard with no
  sidebar graphic, a terminal look (monospace font, Gruvbox palette),
  and a fast typewriter effect (12 ms default) on each page's text that
  completes instantly on click, Space, or Enter. Per-Monitor DPI V2 aware,
  no outer window scrollbars.
- **Payload format** — files are packed into a zstd-compressed archive
  appended after the stub PE, closed by a fixed footer carrying the
  `RUSTPACKER_OVL01` marker, payload offset/length, and SHA-256 hashes
  (whole payload and per file). Packing and extraction both **stream** in
  64 KiB chunks: no source file, no decompressed archive, and no extracted
  file is ever held in memory whole. Extraction rejects corrupt payloads
  and any path that would escape the install directory.
- **Installs** — Per-User, Per-Machine, or Dual-Mode scope (dual-mode
  resolves to per-machine when elevated, per-user otherwise); a
  transactional journal (`install.journal.json`) powers clean rollback
  and the generated `Uninstall.exe`, which is registered in Windows
  Apps & Features. The installer applies the payload, project
  `[[shortcuts]]`, `[[registry]]` values, `[[file_associations]]`, PATH
  entries, the finish-page desktop shortcut, the Apps & Features entry,
  and the at-login entry. Prerequisite and running-process checks run
  before anything is written, and a failed install rolls back every
  change it made.

## Finish page options

The final wizard page offers, conditionally:

- **Create Desktop shortcut** — always, unless hidden in the project file.
- **Launch after setup exits** — always, unless hidden. When the app
  supports the system tray, an indented **Start minimized to system tray**
  sub-option passes the project's tray argument on launch.
- **Launch at Windows login** — only when the app supports the tray. It
  writes the `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` entry
  (recorded in the install journal, so uninstall removes it), with its own
  indented tray sub-option.

## Status

Verified locally: the full workspace test suite, a linked release build
for `x86_64-pc-windows-gnu` (PE32+ x86-64; stub and GUI are GUI-subsystem
binaries), and an end-to-end pack → verify → extract run using the real
Windows stub binary, with `SHA256SUMS.txt` checked by an external tool.

Verified in CI (`.github/workflows/ci.yml`): `cargo fmt --check`,
`cargo clippy -D warnings`, the full test suite and an MSRV (1.85)
build on Linux; on
`windows-latest`, clippy for `x86_64-pc-windows-msvc` (all targets), the
test suite, the release `crt-static` build of all four binaries, a smoke
run of the released CLI (`init` then `validate`), and — since this
version — a resource check that parses the PE resource directory of
`rustpacker-stub.exe` and `rustpacker-gui.exe` and fails if the manifest,
version info, icon or icon group is missing.

Unverified: all Windows 11 **runtime** behaviour (wizard rendering,
typewriter feel, shortcut creation, registry writes, install/uninstall
round-trip, and how the embedded icon and version details actually
appear) — there is no Windows host in the build environment, and CI
compiles and lints the stub and GUI but never runs them. Treat the
installer itself as inspection-verified only until it has been run on a
real machine. When one is available,
[`docs/windows-verification.md`](docs/windows-verification.md) is the
step-by-step pass to run and record here.

Applied by the installer stub: project-file `[[shortcuts]]` (Desktop and
Start Menu), custom `[[registry]]` values (previous values are recorded
and restored on uninstall), `[[file_associations]]` (`Software\Classes`
prog-id + open command, skipped on uninstall when the keys pre-existed),
PATH entries (`path_scope`), prerequisite probes (registry and file), and
running-process checks. Every one of these is journaled and reversed on
uninstall or rollback.

Install safety, as of this version:

- A failed install rolls back every journaled change (files, folders,
  registry values, shortcuts, PATH) instead of leaving a half-installed app.
- Prerequisite and running-process checks run before anything is written.
- A packed installer whose payload is corrupt, whose config is invalid,
  or whose config fails validation refuses to run with an explanatory
  dialog; it no longer falls back to demo mode. Demo mode applies only to
  a stub with no payload at all.
- Per-machine installs offer a UAC re-launch; `dual-mode` projects resolve
  to per-machine when elevated and per-user otherwise.
- Uninstall refuses a journal whose paths lie outside the recorded install
  folder, and the running `Uninstall.exe` is scheduled for deletion on the
  next reboot.
- The install runs on a worker thread, so the window keeps repainting and
  Cancel stays live. Cancelling is cooperative: it is checked before each
  step and before each payload entry, never mid-write, so the journal
  always describes every completed change and the rollback stays exact.
  Extraction reports bytes and drives the progress bar.

Still open: the wizard pages themselves (license text rendering, progress
detail) are minimal, and no automated test exercises Win32 APIs — the
Windows runtime behaviour above remains verified by inspection only.

## Project file

A project is one human-readable TOML file. Generate one with:

```text
rustpacker-cli init --out rustpacker.toml
rustpacker-cli validate rustpacker.toml
rustpacker-cli pack rustpacker.toml --stub rustpacker-stub.exe --portable
```

A worked example lives in `examples/demo/`.

## Build

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --target x86_64-pc-windows-msvc
```

The workspace sets `+crt-static` for `x86_64-pc-windows-msvc` in
`.cargo/config.toml`.

## Releases

Releases are cut by tag: bump `version` in `Cargo.toml`, add the matching
section to `CHANGELOG.md`, then push a `v*` tag.

```text
git tag v0.4.0
git push origin v0.4.0
```

`.github/workflows/release.yml` then refuses to continue if the tag and
`Cargo.toml` disagree, builds the workspace with the same flags CI uses,
re-checks the embedded resources, and **packs the release installer with
RustPacker itself** — using the freshly built stub as the stub and the
freshly built CLI and Builder Studio as the payload. The resulting installer
is then re-read with `verify` and `list` before anything is published, so a
broken `pack` or stub fails the release instead of shipping.

Each release carries exactly two files:

| Asset | What it is |
|---|---|
| `RustPacker-v<version>-x64-Setup.exe` | The installer, packed by RustPacker. Per-user, Windows 11 x64; installs the CLI and the Builder Studio. |
| `RustPacker-v<version>-x64-Portable.zip` | The same files, unpacked — no install needed. |

The release notes are extracted from `CHANGELOG.md` automatically.

## Project Manifest

- Tier: t1b (installer / system utility class) under the Adaptive
  Engineering Contract V4.0.
- Stack: Rust, `windows-sys` (pure Win32), workspace crates
  `rustpacker-core`, `rustpacker-stub`, `rustpacker-cli`, `rustpacker-gui`.
- Release shape: `RustPacker-v{VERSION}-x64-Setup.exe` +
  `RustPacker-v{VERSION}-x64-Portable.zip`, produced by
  `.github/workflows/release.yml` (see [Releases](#releases)).

## License

MIT — Copyright (c) 2026 Sean Shaffer. See `LICENSE`.

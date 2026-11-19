# RustPacker

## TL;DR

RustPacker builds single-file Windows installers: a small native Win32 stub
(`Setup.exe`) with a compressed payload appended to it, plus an optional
portable `.zip` and `SHA256SUMS.txt`. It is 100% offline, has zero
telemetry, targets Windows 11 x64 only, and statically links the C runtime
(`crt-static`) so generated installers need no VC++ redistributable, .NET,
or WebView2 runtime.

## What it does

- **Builder Studio (`rustpacker-gui`)** — dark One Dark Pro studio with
  collapsible side panels and a live preview of the installer wizard,
  including the finish page exactly as the end user will see it.
- **CLI (`rustpacker-cli`)** — every capability from the command line:
  `init`, `validate`, `pack`, `verify`, `list`, `extract`, `schema`, with
  `--json` output and deterministic exit codes (0 ok, 2 usage,
  3 validation, 4 I/O or payload error).
- **Generated installer (`Setup.exe`)** — compact classic wizard with no
  sidebar graphic, a terminal look (monospace font, One Dark Pro palette),
  and a fast typewriter effect (12 ms default) on each page's text that
  completes instantly on click, Space, or Enter. Per-Monitor DPI V2 aware,
  no outer window scrollbars.
- **Payload format** — files are packed into a zstd-compressed archive
  appended after the stub PE, closed by a fixed footer carrying the
  `RUSTPACKER_OVL01` marker, payload offset/length, and SHA-256 hashes
  (whole payload and per file). Extraction rejects corrupt payloads and
  any path that would escape the install directory.
- **Installs** — Per-User, Per-Machine, or Dual-Mode scope; a
  transactional journal (`install.journal.json`) powers clean rollback
  and the generated `Uninstall.exe`, which is registered in Windows
  Apps & Features. The installer applies the payload, the finish-page
  desktop shortcut, the Apps & Features entry, and the at-login entry.

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

Unverified: all Windows 11 runtime behaviour (wizard rendering,
typewriter feel, shortcut creation, registry writes, install/uninstall
round-trip) — there is no Windows host in the build environment. The
release build for the MSVC target with `crt-static` runs in CI
(`.github/workflows/ci.yml`, windows-latest).

Not yet applied by the installer stub (schema and journal support exist,
and the uninstaller already reverses recorded entries of these kinds):
project-file `[[shortcuts]]` beyond the finish-page desktop shortcut,
custom `[[registry]]` values, `[[file_associations]]`, PATH entries, and
the prerequisite / running-process checks. These are the next slice.

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
cargo test
cargo build --release --target x86_64-pc-windows-msvc
```

The workspace sets `+crt-static` for `x86_64-pc-windows-msvc` in
`.cargo/config.toml`.

## Project Manifest

- Tier: t1b (installer / system utility class) under the Adaptive
  Engineering Contract V4.0.
- Stack: Rust, `windows-sys` (pure Win32), workspace crates
  `rustpacker-core`, `rustpacker-stub`, `rustpacker-cli`, `rustpacker-gui`.
- Release shape (when released): `RustPacker-v{VERSION}-x64-Setup.exe` +
  `RustPacker-v{VERSION}-x64-Portable.zip` + `SHA256SUMS.txt`.

## License

MIT — Copyright (c) 2026 Sean Shaffer. See `LICENSE`.

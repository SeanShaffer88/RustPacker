# Changelog

All notable changes to RustPacker are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Builder Studio: portable ZIP builds.** An "Also build portable .zip"
  option next to the finish-page options produces
  `<App>-v<version>-Portable.zip` alongside `Setup.exe`, covered by the
  same `SHA256SUMS.txt`. The export is assembled by core
  `build_portable_zip` — the same function the CLI's `--portable` calls,
  fed by the same pack manifest — so both surfaces ship identical ZIPs
  (AUDIT M14).

## [0.1.0] - 2026-11-19

The first tagged build. RustPacker packs a directory into a single-file
Windows installer: a small native Win32 stub with a compressed, checksummed
payload appended to it, plus an optional portable `.zip`.

### Added

- **Builder Studio** (`rustpacker-gui`) — a native Win32 studio with a
  collapsible project tree, a live preview of the installer wizard, and
  one-click packing.
- **Installer wizard** (`rustpacker-stub`) — a `>_`-styled Win32 wizard with
  a typewriter reveal, per-user or per-machine installs, Desktop and Start
  Menu shortcuts, `[[shortcuts]]`, `[[registry]]`, `[[file_associations]]`,
  PATH entries, prerequisite probes, running-process checks, and
  launch-after-setup / launch-at-login options.
- **CLI** (`rustpacker-cli`) — `init`, `validate`, `pack`, `verify`, `list`,
  `extract`, `schema` and `version`, with `--json` output and documented exit
  codes (0/2/3/4).
- **Transactional installs** — every change is journaled and reversed on
  uninstall or rollback; a refused journal whose paths lie outside the
  install folder; a `Uninstall.exe` scheduled for deletion on the next boot
  because a running program cannot delete itself.
- **Windows resources** — both binaries carry an application manifest
  (`asInvoker`, Per-Monitor-V2 DPI, `longPathAware`, `activeCodePage=UTF-8`,
  `supportedOS` for Windows 10/11 and 8.1), a full `VS_VERSION_INFO`, and an
  embedded icon, so the installer looks like a real application in Explorer,
  the taskbar and Apps & Features.
- **CI** — formatting, clippy under `-D warnings`, and 61 tests on Linux; the
  same plus a `crt-static` release build, a CLI smoke test, and a PE resource
  verification step on `windows-msvc`.

### Changed

- Payload extraction streams through a 64 KiB window with per-entry SHA-256
  verification, so a multi-gigabyte payload no longer has to fit in memory.
- The install runs on a worker thread. The window keeps repainting and Cancel
  stays live; cancellation is cooperative, checked before each step and each
  payload entry, never mid-write, so the rollback journal always describes
  every completed change.
- Extraction reports bytes and drives the progress bar (20–65%), with the
  remaining install steps reporting 65/80/86/89/92/100.

### Fixed

A full audit lives in [`AUDIT.md`](AUDIT.md), which records each finding, its
severity, and whether it is CI-verified or code-only. The high-severity fixes
in this release:

- The uninstaller no longer deletes arbitrary paths from a tampered journal;
  every path is confined to the install directory.
- Decompression is bounded, so a crafted installer can no longer exhaust
  memory.
- A packed installer with a corrupt payload or an invalid project file
  refuses to run with an explanation instead of silently installing a demo
  placeholder.
- A failed install rolls back instead of leaving a half-installed app, and
  uninstall no longer leaves empty directories behind.
- Path sanitisation handles Windows drive, UNC, ADS and reserved-device
  names, and extraction refuses to follow pre-existing symlinks out of the
  destination.
- Archive and ZIP writers error on overflow instead of silently truncating
  lengths and offsets.

### Known limitations

- **Windows runtime behaviour is verified by inspection only.** CI compiles,
  lints, tests and packages these binaries, and it parses the embedded
  resource table — but it never opens the wizard, writes a registry key, or
  creates a shortcut. The wizard and Builder Studio still need a pass on a
  real Windows 11 host, which is what `AUDIT.md` §10 item 10 is about.
- Windows 11 x64 only (`min_windows_build = 22000`); no ARM64 or 32-bit
  builds.
- The folder-`Browse` button on the wizard's directory page is not
  implemented yet (audit finding M11).
- `examples/demo/payload/DemoApp.exe` is a text placeholder, not a real
  executable, so the demo installs a file that will not launch.

[Unreleased]: https://github.com/SeanShaffer88/RustPacker/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/SeanShaffer88/RustPacker/releases/tag/v0.1.0

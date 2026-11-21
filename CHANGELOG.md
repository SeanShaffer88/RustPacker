# Changelog

All notable changes to RustPacker are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.1] - 2026-11-20

### Changed

- **Reference-style Builder Studio.** Replace Gruvbox with the warm-white,
  forest-green, pale-green, and amber palette from the design reference.
  Update navigation, step cards, controls, and the live installer preview.
- **Installer theme defaults.** New projects and reset use the reference
  palette and standard Windows UI typography. Saved project themes stay intact.

## [0.4.0] - 2026-11-21

### Added

- **Gruvbox Builder Studio.** The Studio has been reskinned and
  restructured around the authoring flow: Gruvbox light chrome (cream
  surfaces, deep-green toolbar, green/amber step states), a sidebar
  with the Studio brand and a get-started hint, a four-step project
  flow across the top of the editor (Package identity → Payload →
  User experience → Build, with completed/current/upcoming states), an
  expert-controls strip along the bottom that jumps straight to the
  advanced pages (Shortcuts, Registry, Associations, Prerequisites,
  Processes, Theme), and an "INSTALLER PREVIEW" header over the live
  preview. The preview itself keeps the project's own terminal palette,
  so authors see the wizard exactly as end users will.

- **Silent, scriptable installs.** A packed `Setup.exe` now installs
  with no windows at all: `Setup.exe --silent [--dir <folder>]` uses the
  project's defaults (default install folder unless `--dir` overrides
  it; finish-page checkboxes at their project defaults, including the
  Desktop shortcut and launch-at-login when offered and defaulted on)
  and never launches the app afterwards. Unattended means unattended:
  nothing prompts, and the outcome is the exit code — 0 installed,
  3 payload/install failure (rolled back), 4 blocked by a pre-flight
  check or an unusable folder, 5 administrator rights required. This
  also makes the installer testable end-to-end by automation instead of
  only by hand.

### Fixed

- **The install log survives fresh installs.** `InstallLog::open` now
  creates a missing parent folder; the wizard opens the log before the
  install folder exists, so a first-time install used to end up with no
  `install.log` at all.

### Changed

- **Gruvbox is now the default terminal palette.** New projects (and
  "Reset to Gruvbox" on the Theme page) use the Gruvbox dark palette
  for the installer wizard instead of One Dark Pro. Existing projects
  keep whatever palette they were saved with.
- **Dependency updates (Dependabot).** `toml` 0.8 → 1.2.1 and `zstd`
  0.13 → 0.14.4 (#11, #8); `actions/checkout` and `actions/upload-artifact`
  to v7 (#7, #10). `sha2` stays at 0.10.9: the 0.11 line's dependency tree
  requires a newer Rust than the 1.85 MSRV this project states and
  CI-verifies, and 0.10.9 has no open advisory (#9, closed with details).

## [0.3.0] - 2026-11-20

### Changed

- **Builder Studio redesign.** The Studio now uses the classic
  installer-authoring shell: a File/Build/View/Help menu bar and toolbar,
  a grouped, collapsible section navigation (Project, Installation,
  Requirements, Wizard, Build), one editor page per project section, the
  live wizard preview pinned on the right, and a status bar. Every project
  field is now editable in the Studio — app description, install directory,
  license file, minimum Windows build, PATH scope, finish-page defaults,
  the full terminal palette, and typewriter speed — with add/remove/edit
  list editors for shortcuts, registry entries, file associations,
  prerequisites, and running-process checks. The dark One Dark Pro theme
  and the build/validate/preview behaviour are unchanged. Like the rest of
  the Windows surface, the new shell awaits the Windows 11 runtime pass in
  `docs/windows-verification.md`.
- **Slimmer release downloads.** A GitHub release now ships exactly two
  files: the `Setup.exe` installer and the portable `.zip`. The CLI and
  the Builder Studio are still installed by the installer (and live inside
  the portable ZIP) — they are just no longer separate downloads, and the
  `SHA256SUMS.txt` manifest is dropped with them.

## [0.2.0] - 2026-11-20

The audit-closure release: every P1 item in `AUDIT.md` is now implemented,
and the CLI and Builder Studio produce identical outputs from shared core
code.

### Added

- **Install log (AUDIT M7).** Every install writes `install.log` beside the
  journal: a header and footer around one timestamped line per step,
  rotated to `.1` past 256 KiB. Logging is best-effort — a log failure can
  never fail the install — so a field failure finally leaves something
  behind to diagnose.
- **Wizard confirmations (AUDIT M7).** Cancelling the wizard and running
  the uninstaller ask first (both skipped under `--silent`), instead of
  destroying the wizard or deleting everything without a prompt.
- **Folder Browse and validation (AUDIT M11).** The directory page has a
  Browse button (`IFileOpenDialog`), and the chosen folder is validated
  by core `validate_install_dir`: absolute, no device paths or drive
  roots, no `.`/`..` components, per-component and total length limits,
  no reserved device stems, and never a Windows system root.
- **Builder Studio: portable ZIP builds (AUDIT M14).** An "Also build
  portable .zip" option produces `<App>-v<version>-Portable.zip`
  alongside `Setup.exe`, covered by the same `SHA256SUMS.txt`. The
  export is assembled by core `build_portable_zip` — the same function
  the CLI's `--portable` calls, fed by the same pack manifest — so both
  surfaces ship identical ZIPs.
- **Documentation.** `docs/payload-format.md` specifies the overlay
  footer, archive layout and portable ZIP; `CONTRIBUTING.md` records
  the build/check commands, the MSRV policy and the release process.

### Changed

- **One file-name sanitiser everywhere (AUDIT M13).** Core
  `safe_file_name` now names folders, shortcuts and output files on all
  three surfaces: forbidden/control characters replaced, dots and
  whitespace trimmed from both ends, reserved device stems `_`-prefixed,
  result bounded to 120 characters.
- **Finish-page choices survive (AUDIT L6).** The Builder Studio preview
  no longer re-seeds its checkboxes when the finish model changes state
  moved to core `FinishSelection`, which remembers what the user has
  seen: unseen options carry the project default, seen ones keep the
  user's choice, and hide/show is lossless.
- **Transactional pack outputs (AUDIT L4).** `pack` writes its outputs
  through temp-file + rename with rollback, checksums last, so a failed
  write cannot leave a `Setup.exe` whose `SHA256SUMS.txt` vouches for
  different bytes.
- **Smaller uninstaller (AUDIT L16).** `Uninstall.exe` is written with
  the payload stripped instead of copying the whole installer.
- **Pinned toolchain (AUDIT L9).** CI, releases and local builds use
  Rust 1.100.0 (`rust-toolchain.toml`), and an `msrv` job builds with
  the floor toolchain so the MSRV claim is verified like everything
  else.

### Fixed

- **The MSRV claim was unbuildable.** The workspace claimed Rust 1.70,
  but the dependency tree (indexmap, via serde_json/toml) uses edition
  2024, which cargo older than 1.85 cannot parse. The MSRV is now
  honestly 1.85, and the new `msrv` CI job proves the workspace builds
  with exactly that toolchain.
- **Finish-page edge cases (AUDIT L17).** A whitespace-only
  `startup_tray_arg` now falls back to `tray_arg` instead of silently
  dropping the tray argument, and quote-leading argument fragments are
  rejected rather than embedded into the login `Run` value. New tests
  pin the whole `launch_args`/`login_command` edge matrix.
- Assorted hot-path waste (AUDIT L1): the CRC-32 table is built once,
  hex encoding uses a lookup table, and the wizard redraws its
  typewriter text into a reused buffer.

### Known limitations

- **Windows runtime behaviour is verified by inspection only.** CI
  compiles, lints, tests and packages these binaries, but never opens
  the wizard or the Builder Studio. They still need a pass on a real
  Windows 11 host (`AUDIT.md` §10 item 10).
- Windows 11 x64 only (`min_windows_build = 22000`); no ARM64 or 32-bit
  builds.
- `examples/demo/payload/DemoApp.exe` is a text placeholder, not a real
  executable, so the demo installs a file that will not launch.

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

[Unreleased]: https://github.com/SeanShaffer88/RustPacker/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/SeanShaffer88/RustPacker/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/SeanShaffer88/RustPacker/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/SeanShaffer88/RustPacker/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/SeanShaffer88/RustPacker/releases/tag/v0.1.0

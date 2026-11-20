# RustPacker — Repository Audit

**Date:** 2026-11-19
**Branch audited:** `arena/6b18f778-rustpacker` (from `main` @ `0fd1098`)
**Scope:** Full workspace — `rustpacker-core`, `rustpacker-cli`, `rustpacker-stub`, `rustpacker-gui`, `examples/demo`, CI, docs, dependencies.
**Method:** Static code review of all 6,343 lines of Rust + manifests + CI. No Rust toolchain exists in the sandbox, so the audit itself was static; **the fixes in this branch were then compiled, linted and tested in CI** (`.github/workflows/ci.yml`: `ubuntu-latest` fmt → clippy → test, and `windows-latest` clippy for `x86_64-pc-windows-msvc` → test → release `crt-static` build → CLI smoke test). Line numbers below refer to the **pre-fix** tree.

**Remediation status.** Every finding carries a status line: `FIXED (verified in CI)` means the real pipeline exercised it on this branch; `FIXED (code only)` means it compiles, lints and passes the test suite but its Windows-specific behaviour has not been run; `PARTIAL`/`OPEN` means what it says. CI still never *runs* the GUI or the wizard, so all Windows runtime behaviour (wizard pages, registry writes, shortcuts, install/uninstall round-trip) remains verified by inspection only.

---

## 1. Executive summary

RustPacker is a small, well-structured Windows installer builder: a platform-independent core (schema, payload packer, journal, typewriter) with two thin Windows surfaces (stub wizard + Builder Studio GUI) and a CLI. The design is sound — deterministic packing, footer + per-file SHA-256, traversal rejection for `..`, atomic journal writes, disciplined CLI exit codes, and a genuinely small dependency tree. The README's honesty about what is and isn't verified is a strength.

The audit finds **no show-stopper that invalidates the architecture**, but several **high-severity correctness and safety gaps** that should be fixed before any real release:

1. The uninstaller deletes **any path listed in the journal** without confining it to the install directory.
2. Payload decompression trusts an attacker-controlled `archive_len` for allocation and has **no decompression bomb limit** — a crafted `Setup.exe` can OOM/abort the installer or CLI.
3. A corrupt/tampered `Setup.exe` **silently installs a demo placeholder** instead of erroring.
4. The "transactional journal" does not roll back a failed install, and the overwrite-backup machinery (`OverwriteFile`) is **dead code** — reinstalls destroy previous files with no restore path.
5. Uninstall **leaves empty directories behind** due to a reverse-order recording bug.
6. Path sanitisation is **platform-dependent** and misses Windows drive/UNC/ADS/reserved-name cases when run on Linux; extraction also follows pre-existing symlinks out of the destination.
7. Archive/ZIP writers **silently truncate** lengths/offsets (`as u16`/`as u32`) instead of erroring.
8. `DualMode` scope and most schema features (`require_admin`, prerequisites, process checks, `[[shortcuts]]`, `[[registry]]`, file associations, PATH) are **silently ignored** by the stub — the project file promises behaviour the installer never delivers, with no warning.

Medium-severity issues cluster around validation gaps (unknown fields ignored, most schema sections unvalidated), duplicated/divergent helper logic across CLI/GUI/stub, missing elevation handling, no uninstall confirmation, keyboard-accessibility gaps, and missing Windows manifest/version-info/icon. CI and repo hygiene need hardening (no clippy/fmt, no Linux check, no dependency audit).

**Verdict:** Good foundation for a 0.1.0. As of this branch the P0 findings are fixed in code and the whole workspace builds, lints and tests cleanly in CI on both Linux and Windows-MSVC; what remains before shipping installers is the P1/P2 backlog in §10 and — above all — exercising the wizard and GUI on a real Windows 11 host, which no CI job here does.

---

## 2. Repository at a glance

| Item | Detail |
|---|---|
| Workspace | `Cargo.toml` — 4 members, `edition 2021`, version `0.1.0`, `opt-level=z`, `lto=true`, `codegen-units=1`, `panic=abort`, `strip=true` |
| `rustpacker-core` (1,311 lines) | `schema.rs` (582), `payload.rs` (550), `journal.rs` (201), `typewriter.rs` (149), `lib.rs` (29). **Zero `unsafe`.** Deps: `serde`, `serde_json`, `toml 0.8`, `sha2 0.10`, `zstd 0.13` |
| `rustpacker-cli` (948 + 452 test) | Hand-rolled arg parser, `init/validate/pack/verify/list/extract/schema`, `--json`, exit codes 0/2/3/4. **Zero `unsafe`.** |
| `rustpacker-stub` (1,739 lines) | Win32 wizard + installer/uninstaller. 73 `unsafe` sites. `windows-sys 0.61` only on `cfg(windows)`; non-Windows stub prints a message and exits 2 |
| `rustpacker-gui` (1,539 lines) | Builder Studio, pure Win32. 43 `unsafe` sites. Same `cfg(windows)` pattern |
| Payload format | `[stub PE][zstd archive][72-byte footer: magic RUSTPACKER_OVL01, offset, len, SHA-256, archive_len]`; archive = `u32 count` + per-file `u16 path_len, path, u64 len, 32-byte SHA-256, data`; config at reserved `.rustpacker/config.json` |
| Tests | 29 `#[test]`s: 21 in core (payload round-trip/tamper/traversal/zip, schema finish-model/theme/TOML, journal, typewriter) + 8 CLI integration tests (init→validate, pack→verify/list/extract, JSON, error codes). **No tests for stub/GUI** (Windows-only) |
| CI | `.github/workflows/ci.yml` — `windows-latest`, `cargo test --workspace` + release MSVC build. No clippy/fmt/Linux/audit |
| Deps (from `Cargo.lock`) | 48 packages. Notable: `zstd 0.13.5` (`zstd-sys 2.1.3+zstd.1.5.7`), `toml 0.8.23`, `windows-sys 0.61.2`, `serde 1.0.229`, `sha2 0.10.9`. Small tree — good |
| Docs | `README.md` (honest status section), `examples/demo/` (TOML + 20-byte placeholder `DemoApp.exe` + readme). No `CHANGELOG`, `SECURITY.md`, `CONTRIBUTING` |
| Hygiene | `Cargo.lock` committed ✓, `crt-static` in `.cargo/config.toml` ✓, `.gitignore` = `/target, *.pdb, dist/` only. Single-commit history, no tags |

**What is genuinely good (keep doing this):**

- Clean layering: core is platform-independent and fully unit-tested; stub/GUI/CLI are thin surfaces. `unsafe` is confined to the two Windows binaries.
- Deterministic packing: sorted file walk, fixed ZIP date (1980-01-01), sorted `pack_files` input.
- Defence in depth on the happy path: footer SHA-256 + per-file SHA-256 + `..`/absolute rejection in `sanitise_rel_path` (`payload.rs:69`).
- Atomic journal save (temp + rename, `journal.rs:save`), reverse-order rollback plan, CLI exit-code discipline with JSON/human split (errors to stderr in human mode, to stdout as JSON in `--json` mode).
- Wizard re-entrancy safety: `thread_local!` state + `try_borrow` snapshot pattern (`stub: with_state`) avoids panics from re-entrant `WM_PARENTNOTIFY`.
- Hand-rolled COM vtables for `IShellLinkW`/`IPersistFile` match the documented IDL order; `CoCreateInstance`/`QueryInterface`/`Release` handling is correct.
- README explicitly lists what is unverified and what the stub does not yet apply. That candour is rare and valuable.

---

## 3. Findings — High severity

### H1. Uninstaller deletes arbitrary paths from the journal (no confinement)

**Where:** `crates/rustpacker-stub/src/main.rs` `run_uninstall` (~line 609); `crates/rustpacker-core/src/journal.rs` `InstallJournal::load` / `from_json`.
**Status: FIXED (code only).** `InstallJournal::is_path_confined` rejects anything outside `install_dir`, and `run_uninstall` refuses the whole journal when any entry escapes. `backup_name` is validated through `InstallJournal::backup_path` (bare filename only) before it is joined to the install folder. `validate()` rejects an unknown `schema_version` or an empty `install_dir`. Tests: `confinement_check`, `backup_names_must_be_bare_filenames`.

The uninstaller loads `install.journal.json` beside `Uninstall.exe` and, for every `CreateFile`/`CreateShortcut`/`OverwriteFile`/`CreateDir` entry, calls `remove_file`/`remove_dir` on the stored path **verbatim**. There is no check that the path is inside `journal.install_dir` (or the uninstaller's own directory), no rejection of absolute paths outside it, `..` segments, or UNC paths. `InstallJournal::from_json` also performs no `schema_version`, `app_id`, or path validation.

*Impact:* A tampered or hand-edited journal (or a malicious `Setup.exe` that writes a crafted journal during install) turns `Uninstall.exe` into an arbitrary-file-deletion primitive for the invoking user — and, if the user runs it elevated for a per-machine install, for the machine. Self-harm in the common case, but a real privilege/robustness boundary violation for an installer.

*Fix:* On load, canonicalise (or at minimum lexically normalise) every journal path and refuse to act on any path that is not strictly under `install_dir`. Reject `OverwriteFile.backup_name` values that are absolute or contain `..` (`dir.join(backup_name)` at stub ~line 657 has the same escape). Add a `schema_version` check. Add core tests for confinement.

### H2. Decompression bomb / attacker-controlled allocation in payload decode

**Where:** `crates/rustpacker-core/src/payload.rs` `decode_payload` (~line 295).
**Status: FIXED (verified in CI).** `MAX_ARCHIVE_LEN` (2 GiB) is rejected before any allocation, `Read::take(archive_len + 1)` bounds decompression, and `MAX_FILE_COUNT`/`MAX_PATH_LEN` bound the archive parser. Regression tests cover a lying footer and an oversized archive. Holding the whole archive in memory once remains (see M5).

```rust
let mut archive = Vec::with_capacity(info.archive_len as usize);
decoder.read_to_end(&mut archive)?;
if archive.len() as u64 != info.archive_len { ... }
```

`archive_len` comes straight from the file footer. A crafted footer with `archive_len = 2^63` makes `with_capacity` panic with capacity-overflow (with `panic=abort` in release, the process aborts). Even with a plausible `archive_len`, `read_to_end` on a zstd stream is **unbounded**: a small payload can decompress to gigabytes (zip bomb), OOM-killing the stub wizard, `verify`, `list`, or `extract`. `parse_archive`'s `count.min(1_000_000)` cap only limits the file-count pre-allocation, not the decompressed size or per-file `data_len` allocations (each `take(data_len).to_vec()` copies).

*Fix:* Cap `archive_len` against a sane maximum (e.g. reject if `> payload_len * ratio` or an absolute ceiling), use `decoder.take(limit)` / `Read::take` so decompression stops at `archive_len + 1`, and stream per-file data instead of holding the whole archive + every file in memory. Add a regression test with a lying footer and a bomb payload.

### H3. Corrupt/tampered `Setup.exe` silently installs a demo instead of erroring

**Where:** `crates/rustpacker-stub/src/main.rs` `load_config` (~line 165).
**Status: FIXED (code only).** Demo mode now applies only when `find_payload` finds nothing at all; a present-but-unreadable payload, a missing config or an invalid config returns an error, and the stub shows it and exits non-zero.

Every failure mode — unreadable exe, missing payload, `read_manifest` error, missing config, `ProjectConfig::from_json` error — falls back to `demo_config()` with `packed=false`. The wizard then installs a placeholder `README-RustPackerDemo.txt` and reports success.

*Impact:* A bit-rotted, truncated, or maliciously modified installer appears to succeed while installing nothing real. The user gets no error, no log, and an ARP entry for the wrong app. This defeats the payload integrity story (footer hash) at the exact moment it matters.

*Fix:* Distinguish "no payload" (bare stub → demo mode, clearly labelled) from "payload present but invalid" (hard error, message box + non-zero exit). Never fall back to demo when `find_payload` succeeds but parsing fails.

### H4. No transactional rollback on install failure; overwrite backup is dead code

**Where:** `crates/rustpacker-stub/src/main.rs` `perform_install` (~line 484); `crates/rustpacker-core/src/journal.rs` `JournalAction::OverwriteFile`.
**Status: FIXED (code only).** The journal is recorded *before* `extract_payload` writes anything, so a mid-extract failure is still undoable. `OverwriteFile` is now really produced (pre-existing files are copied to `install.journal.json.<hash>.bak`) and `rollback_journal` restores them; a failed install runs the rollback plan and reports what it could not undo. The README claims now match the code.

`perform_install` extracts files, copies the uninstaller, writes ARP, and only **then** saves the journal. If extraction fails midway (disk full, permission, corrupt entry), partial files remain on disk with **no journal** — uninstall cannot clean them up. Separately, every extracted file is recorded as `CreateFile` even when it overwrites a pre-existing file; `OverwriteFile { backup_name }` is never constructed anywhere in the stub (grep confirms only the uninstall arm references it). The module docs promise "nothing it replaced is lost silently" and "overwritten files keep their previous data" — neither is true.

*Impact:* Failed installs orphan files; upgrades/reinstalls destroy previous versions with no restore; rollback after failure is impossible. The "transactional" claim in the README overstates the implementation.

*Fix (pick a scope):* Minimum — save the journal incrementally (after each step, or at least before extraction with a "incomplete" marker) and delete partial output on error. Proper — before overwriting an existing file, copy it to a backup name and record `OverwriteFile`; restore on rollback/uninstall. Update README/docs to match whichever behaviour ships.

### H5. Uninstall leaves empty directories behind (reverse-order bug)

**Where:** `crates/rustpacker-stub/src/main.rs` `perform_install` (~lines 519–532) vs `run_uninstall` `CreateDir` arm (~line 660).
**Status: FIXED (code only).** `rollback_journal` collects `CreateDir` entries and removes them deepest-first *after* every file, and reports the failures instead of ignoring them; `remove_dir` still only succeeds on empty folders, so a user's own folder survives.

For each extracted file the installer records its parent chain **outward** (e.g. for `a/b/c.txt`: records `a/b`, then `a`). The journal order is therefore `[install_dir, a/b, a]`. Rollback walks in reverse — `[a, a/b, install_dir]` — so `remove_dir(a)` runs while `a/b` still exists and fails (only empty dirs are removed), then `a/b` is removed, then `install_dir` also fails because `a` remains. Net result: an empty `a/` (and the install dir itself) is left behind, while the uninstaller still reports success (the `remove_dir` failures are ignored with `let _ =`).

*Fix:* Record directories **deepest-first** (or sort the rollback plan by path depth descending for `CreateDir`), and surface/report leftover-directory failures instead of ignoring them. Add a core-level test that simulates the recording order.

### H6. Path sanitisation is platform-dependent; symlink escape on extract; symlink-following pack

**Where:** `crates/rustpacker-core/src/payload.rs` `sanitise_rel_path` (69), `extract_payload` (343), `collect_files` (248).
**Status: FIXED (verified in CI on Linux; the Windows parse path is code only).** Sanitisation is explicit and host-independent: `:` (drive/ADS), UNC/absolute prefixes, empty segments, `.`/`..` and reserved device names (`CON`, `NUL`, …) are rejected. Extraction canonicalises the destination and each parent and refuses writes that escape it. Packing uses `symlink_metadata`, skips symlinks (with a warning) so a loop can neither recurse nor pack outside content, and rejects duplicate/case-colliding entries.

- `sanitise_rel_path` uses `Path::components()`, whose parsing differs by host OS. On Linux, `"C:/evil.exe"` parses as `Normal("C:")` + `Normal("evil.exe")` and **passes**; on Windows it is a `Prefix` and is rejected. The same payload therefore validates differently depending on where `extract` runs. Windows-specific hazards that pass on both hosts: drive-relative `"C:evil.exe"`, UNC `"//server/share/evil"`, ADS `"file.txt:stream"`, reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`…), trailing dots/spaces (`"file.txt."`, `"file.txt "` — Windows strips these, enabling collisions/overwrites).
- `extract_payload` does `dest_dir.join(rel)` + `create_dir_all(parent)` + `File::create` with **no symlink check**. If `dest_dir/sub` is a pre-existing symlink to outside (or a payload entry creates a symlink — not possible in this format, but a pre-seeded dest can), writes escape `dest_dir`. There is no `canonicalize` + prefix check anywhere in the repo.
- `collect_files` (and its CLI duplicate) uses `path.is_dir()` / `is_file()`, which **follow symlinks**, with no cycle detection. A symlink loop in the source tree recurses until stack overflow / path-length failure; a symlink to a file outside the source silently packs outside content. Broken symlinks are silently skipped.

*Fix:* Make sanitisation OS-independent: reject any path containing `:` (kills drive/ADS), leading `/` or `\`, empty components beyond a single trailing slash, `.`/`..` segments (already), reserved names, and trailing dot/space segments — explicitly, not via `Path::components()`. On extract, after creating parents, verify the canonicalised parent is still under canonicalised `dest_dir` (or open with `O_NOFOLLOW`-equivalent semantics). On pack, use `symlink_metadata` and either skip symlinks or record them explicitly; detect cycles via visited canonical paths.

### H7. Silent length/offset truncation in archive and ZIP writers

**Where:** `crates/rustpacker-core/src/payload.rs` `build_archive` (94), `write_stored_zip` (377).
**Status: FIXED (verified in CI).** `build_archive`, `write_stored_zip` and `pack_files` return errors when a path, file count, entry count, entry size or offset exceeds its field width, with boundary tests.

- `build_archive`: `path_bytes.len() as u16` truncates paths > 65,535 bytes; `files.len() as u32` truncates file counts > 4B. No validation in `pack_files`.
- `write_stored_zip`: `data.len() as u32` (×2 per entry), `name.len() as u16`, `out.len() as u32` (offset), `central_len/offset as u32`, `entries.len() as u16` — all truncate silently. A >4 GiB file, >65k-entry payload, or >4 GiB total ZIP produces a **corrupt archive with no error**.

*Fix:* Return `Result` and error when any length exceeds its field width (or switch the portable export to a real ZIP writer with ZIP64). At minimum, validate in `pack_files`/`write_stored_zip` and add boundary tests. The same class of bug exists in `find_payload`'s `*e as usize` comparison (fine on x64, wrong on 32-bit — the project is x64-only, but the core is presented as platform-independent).

### H8. `DualMode` and most schema features are silently ignored by the stub

**Where:** `crates/rustpacker-stub/src/main.rs` `default_install_dir` (209), `perform_install` (492), `run_wizard` (1531); `crates/rustpacker-core/src/schema.rs`.
**Status: FIXED (code only).** `InstallScope::resolve(elevated)` implements dual mode (per-machine when elevated, per-user otherwise); the stub applies `require_admin` (with a UAC re-launch prompt), `min_windows_build`, `prerequisites` (registry and file probes), `process_checks`, `[[shortcuts]]`, `[[registry]]`, `[[file_associations]]` and `path_scope`, and journals everything it writes.

- `per_machine = matches!(scope, PerMachine)` — **`DualMode` behaves exactly like `PerUser`** for install dir and ARP root. GUI and CLI happily accept `DualMode`; the stub never distinguishes it.
- `require_admin`, `min_windows_build`, `prerequisites`, `process_checks`, `[[shortcuts]]` (beyond the finish-page desktop shortcut), `[[registry]]`, `[[file_associations]]`, `path_scope` are **never read** by the stub (grep: only `license_file` is referenced, and only to show its *path string* on the License page — the file's contents are never displayed). The README discloses this ("Not yet applied… These are the next slice"), but the stub gives the user **no warning** that their configured checks/policies were skipped.

*Impact:* A packager who sets `require_admin = true` or a prerequisite gets a silent no-op. Per-machine installs fail late (HKLM/`ProgramFiles` permission error mid-install) instead of elevating or failing fast with a clear message.

*Fix:* Short term — emit a visible warning (wizard page + status) listing ignored settings, and fail fast with a clear message when `require_admin`/`PerMachine` is requested without elevation. Long term — implement the checks (OS build via `RtlGetVersion`, registry/file probes, process wait) or remove the fields from the schema until they exist. Decide what `DualMode` means (prompt for scope? install per-user by default?) and implement or remove it.

### H9. GUI build filename uses the raw version string (traversal / invalid names)

**Where:** `crates/rustpacker-gui/src/main.rs` `build_setup` (~line 1503).
**Status: FIXED (code only).** The GUI builds filenames through core `safe_file_name` (reserved device names, dots and spaces handled), the same function the CLI uses.

```rust
format!("{}-v{}-Setup.exe", sanitize_filename(&app.cfg.app.name), app.cfg.app.version)
```

The version is **not sanitised** (the CLI correctly applies `safe_file_name` to both name and version, `cli: cmd_pack`). A version containing `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|` produces an invalid filename or — with `/`/`\` — writes outside the output directory via `out_dir.join(&setup_name)`. `sanitize_filename` itself (GUI) also differs from the CLI's `safe_file_name` (GUI allows spaces/dots, replaces others with `_`; CLI replaces forbidden chars with `-` and trims dots/spaces), so CLI and GUI produce different filenames for the same project.

*Fix:* Share one `safe_file_name` implementation in core; apply it to name *and* version in both CLI and GUI; validate `app.version` in `ProjectConfig::validate` (non-empty, no path separators).

### H10. `main_exe` is not validated as a bare filename — launch/shortcut/Run can escape the install dir

**Where:** `crates/rustpacker-core/src/schema.rs` `validate` (417, only checks non-empty); `crates/rustpacker-stub/src/main.rs` `finish_and_close` (~line 1272: `install_path.join(&cfg.app.main_exe)`).
**Status: FIXED (verified in CI).** `schema::is_bare_filename` rejects separators, drive/ADS colons, control characters, wildcards, trailing dots/spaces and reserved device names; `validate()` enforces it for `main_exe` and `FileAssociation.open_with`, and `pack` additionally verifies `main_exe` is present in the payload.

`main_exe = "../evil.exe"`, `"C:\\Windows\\evil.exe"`, or an absolute path passes validation. `Path::join` with an absolute path **replaces** the base, so the desktop shortcut, the `HKCU\...\Run` value, and the post-install `ShellExecuteW` launch all point outside the install directory. The packer also never checks that `main_exe` actually exists in the payload, so a typo produces a "successful" install whose Launch button does nothing (the `ShellExecuteW` result is ignored).

*Fix:* Validate `main_exe` as a single `Normal` component (no separators, no `..`, no drive/ADS, no reserved name) and verify at pack time that it is present in the packed file list. Surface `ShellExecuteW` failures to the user.

---

## 4. Findings — Medium severity

### M1. Unknown project-file fields are silently ignored (typos pass validation)

**Status: FIXED (verified in CI).** `#[serde(default, deny_unknown_fields)]` on the config structs, so a misspelled key fails `validate`/`pack` instead of silently defaulting.
No `#[serde(deny_unknown_fields)]` anywhere in `schema.rs`. A typo like `suports_tray = true` or `sorce_dir = "x"` parses cleanly, fills defaults, and validates — the user believes the setting applied. *Fix:* add `deny_unknown_fields` to `ProjectConfig` and all nested structs (or at minimum the top level), with a test that a misspelled key fails `validate`/`from_toml`.

### M2. `ProjectConfig::validate` covers only a fraction of the schema

**Where:** `schema.rs:417`. Checked: name/version/app_id/main_exe non-empty, source_dir non-empty, typewriter range, tray_arg when tray enabled, path_scope enum, file-association extension shape, registry root enum, theme colours. **Not checked:** `publisher`/`description` (fine to be empty, but inconsistent), `install.default_dir` (empty/garbage accepted), `license_file` existence, `min_windows_build` range, `startup_tray_arg`, `shortcuts` (entirely unvalidated — empty name/location/target, bogus locations), `registry` key/name/value emptiness, `file_associations` prog_id/open_with/duplicates, `prerequisites` (empty name/message, both probes empty = always-pass prereq), `process_checks` (empty process_name). *Fix:* validate each section; check `license_file`/`source_dir` existence at pack time (GUI already checks source existence — CLI does not, see M14).
**Status: FIXED (verified in CI).** Validation now covers `app.publisher`/`description`, `install.default_dir`, the whole `[[shortcuts]]`/`[[registry]]`/`[[file_associations]]`/`[[prerequisites]]`/`[[process_checks]]` surface, and `license_file`/`source_dir` existence at pack time.

### M3. Reserved config entry can be bypassed with `./` prefix; duplicates not handled

**Status: FIXED (verified in CI).** Entry paths are normalised (leading `./`, doubled separators) before the reserved-entry check and before storage, and duplicate normalised paths are rejected case-insensitively.
`pack_files` rejects `path == ".rustpacker/config.json"` (after `\`→`/` normalisation) but `"./.rustpacker/config.json"` passes the check, is stored verbatim, and is then treated as a **regular file** by `read_manifest` (exact-match comparison) and written to disk by `extract_payload` (sanitise strips the `./`). Duplicate payload paths in general (including case-variants like `A.txt`/`a.txt` on Windows) are not detected — later entries silently overwrite earlier ones on extract. *Fix:* normalise (strip `./`, collapse `//`) before the reserved-entry check and before storing; reject duplicate normalised paths (case-insensitively on Windows).

### M4. Pack-time checks are missing (stub PE, main_exe, license file)

**Status: FIXED (verified in CI).** `pack` requires `MZ` + a plausible PE header in the stub, verifies `main_exe` is in the payload, errors when `license_file` is missing, and embeds the license *text* (≤256 KiB) so the License page can show it.
`cmd_pack` accepts any file as `--stub` (tests use `MZ` + zeros — there is no PE validation at all), never verifies `main_exe` is in the payload, and never checks `license_file` exists. A non-PE stub or missing main exe packs "successfully" and fails at install/launch time. *Fix:* validate the stub starts with `MZ` + a plausible PE header (or at least warn), verify `main_exe` ∈ packed entries, verify `license_file` exists when set.

### M5. Whole-payload-in-memory, multiple times over

**Status: PARTIAL.** The portable ZIP no longer re-reads the source tree and the payload decode is bounded, but the archive is still held in memory whole and the stub still decodes it more than once. Documented, not fixed.
`pack_directory` reads every file fully into memory, then builds the archive, compresses it, and concatenates stub+payload+footer — 3–4× payload size resident. The stub decompresses the payload **three times** (`load_config` → `read_manifest`, `perform_install` → read exe, `extract_payload` → decode again). CLI `pack --portable` re-walks and re-reads the entire source tree a second time (`collect_source_files`). Large payloads (hundreds of MB+) will OOM on modest machines. *Fix:* stream where practical (or at least document the limit); have the stub load the config once and reuse the decoded archive for extraction; have CLI reuse `pack_directory`'s entries for the portable ZIP instead of re-reading.

### M6. No elevation handling for per-machine installs

**Status: FIXED (code only).** A per-machine install (or `require_admin`) checks elevation early and offers a UAC re-launch instead of failing mid-install.
Per-machine scope writes to `%ProgramFiles%` and `HKLM` with no admin check, no `runas` relaunch, and no pre-flight failure. The user sees a mid-install registry error. `require_admin` is likewise unenforced (H8). *Fix:* check elevation early (`CheckTokenMembership` / `IsUserAnAdmin` equivalent via `windows-sys`), fail fast with "run as Administrator" guidance, or relaunch elevated.

### M7. No confirmation before uninstall or cancel; no logging

**Status: PARTIAL.** Uninstall asks for confirmation first (skipped under `--silent`). The wizard cancel confirmation and an install log are still missing.
`run_uninstall` deletes everything immediately (only a result message box afterwards). `Cancel` destroys the wizard with no confirmation even after a successful install. There is no log file — field failures are undebuggable (message-box text only). *Fix:* `MB_YESNO`/`MB_OKCANCEL` confirmations (respecting `--silent`), plus an optional install log (`install.log`) recording steps, journal path, and errors.

### M8. Keyboard accessibility: Enter/Space cannot advance the wizard

**Status: FIXED (code only).** `WM_KEYDOWN` handles Space/Enter on the main window: the first press finishes the typewriter, the next advances the page (Welcome/License/Directory/Finish), so the wizard can be completed by keyboard.
`show_page` ends with `SetFocus(hwnd)` (main window). `WM_KEYDOWN` for Space/Return only skips the typewriter when incomplete; when complete it falls to `DefWindowProc`, which does nothing for the main window — the focused `Next` button never receives the key. Keyboard-only users cannot proceed. *Fix:* set a default button (`DM_SETDEFID`) or forward Return to `ID_NEXT` when the typewriter is complete; don't steal focus from controls; add visible focus rectangles.

### M9. Install runs synchronously on the UI thread

**Status: OPEN.** The install still runs inline in the message handler; the window is unresponsive while extracting.
`show_page(Installing)` calls `run_install_on_page` → `perform_install` inline in the message handler. The window freezes for the whole extraction (no painting, no Cancel, close button unresponsive until done; queued timer messages burst afterwards). *Fix:* run the install on a worker thread and post progress/completion messages, or at minimum pump messages during extraction.

### M10. License page shows the file *path*, not the license

**Status: FIXED (code only).** `install.license_text` is derived at pack time from `license_file`, capped at 256 KiB and never serialised, and the License page renders the text. `validate` exits 0 but warns when a project has neither a usable `license_file` nor `license_text`.
`page_body(License)` interpolates `cfg.install.license_file` (a path string) into the body text. The license file is never read or displayed, and `license_file` is never embedded in the payload. Users "accept" a license they cannot read. *Fix:* embed the license text in the config/payload and render it (read-only edit control), or remove the page until implemented.

### M11. Install-directory page accepts any path

**Status: PARTIAL.** Emptiness is still the only check on the folder page and there is still no Browse button; `create_dir_all` failures now surface as install errors instead of being swallowed.
Only emptiness is checked. `C:\Windows\System32`, relative paths, UNC paths, and paths with invalid characters are accepted; `create_dir_all` then either fails mid-install or writes into a system location. There is no Browse button. *Fix:* validate (absolute, writable, not a Windows system dir, length limits), add Browse, and warn when the target is outside the default root.

### M12. CLI argument-parser quirks

**Status: FIXED (verified in CI).** `--json` is a normal flag (only stripped before a `--` terminator; everything after it is positional), `--` is supported, `--opt=value` works, duplicate options and missing values are errors, and `version` is documented in `HELP`.
`real_main` strips **every** `--json` token globally before dispatch, so `--json` cannot be a filename/value and `pack --stub --json` mis-parses. There is no `--` terminator (files starting with `-` are unpassable), duplicate options silently take the last value, and `--out --force` consumes `--force` as the value. `version` is implemented but absent from `HELP`. `schema --json` prints TOML, not JSON (arguably fine, but undocumented). *Fix:* parse `--json` as a normal flag per-command, support `--`, error on duplicates, document `version`.

### M13. Three divergent filename sanitisers

**Status: PARTIAL.** CLI and GUI both use core `safe_file_name` (reserved names, trimmed dots/spaces, length-bounded); the stub's `sanitize_folder` still differs because it names a folder rather than a file.
CLI `safe_file_name` (`-` replacement, trims dots/spaces), stub `sanitize_folder` (`_` replacement, trims trailing dots), GUI `sanitize_filename` (`_` replacement, allows space/dot/dash/underscore). Same app name → different folder vs. filename spellings; none handles reserved device names (`CON`, `NUL`…) or length limits. *Fix:* one core function, used everywhere, with reserved-name + length handling.

### M14. CLI and GUI validation/build behaviour diverges

**Status: PARTIAL.** Both surfaces now share core validation and `safe_file_name`; the GUI still has no portable-ZIP option.
GUI `validation_errors` checks source-dir existence + output-dir non-empty; CLI `validate` checks neither. GUI has no portable-ZIP option; CLI does. GUI prefers a sibling `rustpacker-stub.exe` over the typed stub path (undocumented precedence); CLI prefers project-dir over cwd only when the path exists. *Fix:* share validation + path-resolution in core; document stub precedence; add portable option to GUI or document its absence.

### M15. Journal save is not durable; load has no version check

**Status: FIXED (verified in CI).** `save` fsyncs the temp file (and the directory, best effort), deletes the temp file on failure, and `load`/`from_json` reject an unknown `schema_version`.
`save` writes temp + rename but never `fsync`s file or directory (crash → empty/missing journal), leaves the `.tmp` file behind on failure, and `load`/`from_json` accept any `schema_version`. *Fix:* `fsync` before rename, clean up temp on error, reject unknown `schema_version`.

### M16. Uninstall of file associations deletes pre-existing user data

**Status: FIXED (code only).** The journal records `previous_existed`, and uninstall only deletes the `Software\Classes` keys when this install created them; registry values already restored their previous value.
The `FileAssociation` arm deletes `HKCU\Software\Classes\<ext>` and `\<prog_id>` trees unconditionally — if the user already had an association for `.txt`, uninstall destroys it with no restore (unlike `RegistryValue`, which at least restores `previous_value`). `RegistryValue.previous_value` is `Option<String>` only, so non-string values can't be represented. *Fix:* record previous state (or at least "existed before") and restore/delete accordingly; document REG_SZ-only support.

### M17. Tray-argument handling is inconsistent; Run-value quote injection

**Status: FIXED (verified in CI).** Launch arguments and the Run value both derive from `effective_startup_tray_arg`/`tray_arg` the same way, and `login_command` returns `None` instead of building a command line from a path containing `"` or control characters.
`launch_args` returns `tray_arg` as a **single** argv entry (the stub then wraps it in quotes if it contains a space → `"--tray --at-login"` as one argument), while `login_command` embeds the same string as a **command-line fragment** (unquoted → two arguments). Multi-word tray args behave differently for launch vs. login. Separately, `login_command` interpolates `exe_path` inside quotes without escaping embedded `"` — a crafted install dir/`main_exe` injects extra arguments into the `Run` value. *Fix:* define `tray_arg` as either a single arg or a fragment (not both), escape/validate `exe_path`, and add tests for multi-word args.

### M18. Missing Windows manifest, version info, and icon

**Status: OPEN.** No `build.rs`, no manifest/version resource, no icon. DPI awareness is still set programmatically.
No `.manifest`, no `VS_VERSION_INFO`, no icon resource for stub or GUI (`hIcon` null, `LoadIcon` never called). Consequences: no `supportedOS`/`longPathAware`/`activeCodePage` declarations, no file/product version in Explorer, default icon in Explorer/ARP/taskbar. DPI awareness is set programmatically (good), but a manifest is still the correct place for compatibility declarations. *Fix:* add a build script (`build.rs` + `embed-manifest`-style or `windows-sys` resource) with manifest + version info + icon for both binaries.

---

## 5. Findings — Low / informational

| ID | Finding | Location | Status |
|---|---|---|---|
| L1 | `crc32` rebuilds its 256-entry table on **every call**; `sha256_hex` does `format!` per byte; `Typewriter::visible_text` allocates per 12 ms tick; CLI `setup_report` re-hashes the payload after `read_manifest` already did. All wasteful, none incorrect. | `payload.rs:432,60`, `typewriter.rs`, `cli: setup_report` | PARTIAL — `crc32` table built once via `OnceLock`; other items unchanged |
| L2 | `collect_files` duplicated verbatim in core and CLI; core version has dead `let _ = root;` param; broken symlinks silently skipped. | `payload.rs:248`, `cli: collect_files` | PARTIAL — the pack path no longer re-reads the tree; CLI still has its own walker |
| L3 | Duplicate/case-colliding payload paths not detected (see M3). | `payload.rs: pack_files/extract_payload` | FIXED — duplicates rejected at pack time |
| L4 | `pack` leaves partial outputs (Setup without sums, etc.) when a later write fails. No cleanup/transaction. | `cli: cmd_pack` | OPEN |
| L5 | Stub `get_text` truncates at 2,048 chars (long install paths silently cut); GUI file dialog caps at 1,024 WCHARs; magic numbers for font params (`CreateFontW(-16,0,0,0,400,0,0,0,1,0,0,5,49,…)`) and window styles instead of named constants. | `stub: get_text, run_wizard`, `gui: file_dialog` | OPEN |
| L6 | GUI: preview checkboxes re-seed (losing user toggles) whenever the finish *model* changes; elevated-surface colour hardcoded instead of from theme; `RegisterClassW` result ignored; window-creation failure exits silently with no message. | `gui: refresh_preview, color_control, run` | OPEN |
| L7 | CLI `version` command undocumented in `HELP`; `schema --json` prints TOML. | `cli: dispatch, cmd_schema` | FIXED — `version` and `schema --json` documented |
| L8 | `.gitignore` doesn't cover `*.exe`/`*.zip`/`SHA256SUMS.txt`/`rustpacker.toml` in repo root (default `--out-dir` is `.`); no `rustfmt.toml`/`clippy.toml`/`.editorconfig`; no `CHANGELOG`/`SECURITY.md`; single-commit history, no tags/releases. | repo root | OPEN |
| L9 | CI: no `cargo clippy`, no `cargo fmt --check`, no Linux/macOS `cargo check` (despite "host builds pass" claims), no `--locked`, no caching, unpinned `stable` toolchain, no artifact upload, no `cargo audit`/`cargo deny`. | `.github/workflows/ci.yml` | PARTIAL — Linux fmt/clippy/test + Windows clippy/test/release/smoke added; no `--locked`, cache, pinned toolchain or audit |
| L10 | Dependencies are few and current-ish (`serde 1.0.229`, `sha2 0.10.9`, `windows-sys 0.61.2`), but `toml 0.8.23` and `zstd-sys 1.5.7` should be tracked for advisories; no `deny.toml`/Dependabot. **No audit was run in this sandbox** (no toolchain) — run `cargo audit` + `cargo deny check` in CI. | `Cargo.lock` | OPEN — not run; needs CI `cargo audit` |
| L11 | Release profile (`panic=abort`, `strip`, `lto`, `opt-z`) minimises size but turns any panic into an abort (exit 134, undocumented) and strips symbols needed for field diagnostics. Consider `panic=unwind` for debug builds or at least document the behaviour. | `Cargo.toml` | OPEN |
| L12 | Test temp dirs use `std::process::id()` (+ a counter in CLI tests, but **no counter in core tests**) — parallel test binaries sharing a PID namespace could collide; core tests also don't clean up on panic. No tests for: Windows sanitise cases, bombs/lying footers, truncation boundaries, journal traversal, schema gaps, `parse_args` edges, `safe_file_name` edges. | `payload.rs` tests, `journal.rs` tests, `cli.rs` tests | PARTIAL — 53 tests now; still no Windows-API tests |
| L13 | `examples/demo/payload/DemoApp.exe` is a 20-byte text placeholder (`MZ-demo-placeholder`), not a PE. Packing the demo with a real stub yields a Setup that installs a text file as the "exe". Docs should say so explicitly. | `examples/demo/` | OPEN |
| L14 | Typewriter reveals by `char`, splitting grapheme clusters/emoji ZWJ sequences; `reset()` cannot change the interval. Cosmetic only. | `typewriter.rs` | OPEN |
| L15 | `InstallJournal::save` temp name via `with_extension("json.tmp")` turns `install.journal.json` into `install.journal.tmp` — works, but surprising; two concurrent saves would race. | `journal.rs: save` | FIXED — fsync + temp cleanup + schema-version check (`.json.tmp` naming remains) |
| L16 | Stub copies the **entire** `Setup.exe` (payload included) to `Uninstall.exe`, duplicating the payload on disk. Harmless but wasteful; a payload-free uninstaller stub would halve the footprint. | `stub: perform_install` | OPEN |
| L17 | `FinishScreenModel`/`TrayConfig` logic is well-tested, but `launch_args`/`login_command` edge cases (empty `tray_arg` with tray enabled is validated, but whitespace-only `startup_tray_arg` fallback uses `trim().is_empty()` while `login_command` embeds the untrimmed string) deserve a few more tests. | `schema.rs` | PARTIAL — `login_command` escaping covered by validation |
---

## 6. Per-crate notes

### `rustpacker-core` — strongest crate; fix the payload boundary
Schema, journal, and typewriter are clean, well-documented, and well-tested. The payload module is where all the high-severity issues live (H2, H6, H7, M3, M5). The format itself (footer + hashes + sorted deterministic archive) is a good design; the implementation needs bounds, caps, and OS-independent validation. `TerminalTheme` validation and `FinishScreenModel` are exemplary.

### `rustpacker-cli` — solid UX, hand-rolled parser needs hardening
Exit-code discipline, JSON/human duality, and the `init → validate → pack → verify/list/extract` loop are all well done and well tested (including the "resolve source relative to project file, not cwd" case). Weak spots: global `--json` stripping (M12), missing pack-time checks (M4), double-read for portable (M5), partial outputs (L4), and the duplicated `collect_files` (L2). The hand-rolled JSON escaper is correct (handles `"`, `\`, control chars) but `serde_json` is already a dependency via core — prefer it.

### `rustpacker-stub` — largest risk surface; unverified on Windows
The wizard structure (pages, typewriter timer, finish-page model rendering) is thoughtful and the re-entrancy handling is careful. But the install/uninstall core has H1/H3/H4/H5/H8/H10, the UI thread blocks during install (M9), keyboard navigation is broken (M8), and none of it can be tested in this sandbox or (per README) on a real host yet. **This crate needs a Windows 11 test pass before anything else ships.** The COM shortcut code and registry helpers are correct as far as static review can tell.

### `rustpacker-gui` — good preview architecture, smaller bugs
The live-preview-via-`FinishScreenModel` design (single authority for checkbox visibility) is nice. Issues are mostly M14/M13/H9-scale plus L6 polish. The GUI edits only a subset of the schema (by design — unedited fields round-trip untouched, which is correct), but users can't edit description/default-dir/license/admin/build/typewriter/theme/shortcuts/registry from the GUI at all. Either expand the GUI or clearly label it "subset editor — edit TOML for advanced fields".

---

## 7. Security summary

| Threat | Status |
|---|---|
| Path traversal (`../`, absolute) in payload | **Blocked** for `..`/absolute on the host OS (tests cover it), but **bypassable in principle** via Windows-specific forms on Linux + symlink escape (H6) |
| Corrupt/tampered payload | Footer + per-file SHA-256 **detected** — but stub **ignores** the failure and installs demo (H3) |
| Decompression bomb / OOM | **Unprotected** (H2) |
| Journal-driven arbitrary deletion | **Unprotected** (H1) |
| Registry/Run injection via quotes | **Partially unprotected** (M17) |
| Network / telemetry / supply-chain exfil | **Clean** — no sockets, no HTTP, no `Command` spawning in shipped code (only tests spawn the CLI), no telemetry. Offline claim holds |
| Secrets in repo | **None found** — no tokens, keys, or credentials |
| `unsafe` | Confined to stub/GUI Win32 interop; core/CLI are 100% safe Rust. COM vtables verified against IDL order |
| Elevation / privilege | **Missing** — per-machine installs fail late instead of elevating (M6) |

---

## 8. Testing & CI assessment

- **Coverage (as it stands after remediation):** **53 tests** (the 29 reviewed here plus regression tests added with the fixes — payload limits/lying footer, sanitisation, duplicates, journal confinement and backup names, `is_bare_filename`, license text, CLI `--`/`--opt=value`/duplicate-option parsing). All of them run and pass in CI on both `ubuntu-latest` and `windows-latest`.
- **Coverage (at review time):** 29 tests. Core: payload round-trip, tamper rejection, traversal rejection, `pack_directory`, ZIP structure + CRC vector, theme defaults, finish-model matrix (tray on/off, hidden options, startup-arg override), TOML round-trip + validation, newer-schema rejection, journal rollback order + JSON + disk round-trip, typewriter timing/skip/unicode/reset. CLI: help, init (+overwrite guard + `--force`), schema-validates, full pack→verify/list/extract round-trip from a foreign cwd, pack JSON shape, invalid-project error listing (human + JSON), pack-rejects-invalid, usage-vs-IO exit codes. This is **above-average coverage for a 0.1.0**.
- **Gaps:** everything in L12, plus no stub/GUI tests at all (understandable — Windows-only — but a Windows CI job running the stub in demo mode + a scripted install/uninstall round-trip would close the biggest verification gap the README calls out).
- **CI:** Windows-only at review time (test + release build). **Now:** a `linux` job (`cargo fmt --all -- --check` → `cargo clippy --workspace --all-targets -- -D warnings` → `cargo test --workspace`) and a `windows-msvc` job (clippy for `x86_64-pc-windows-msvc` → tests → release `crt-static` build → list binaries → run the CLI against a generated project). That pipeline is what caught and cleared the compile errors, clippy warnings and formatting drift in this branch. Still missing: `--locked`, caching, a pinned toolchain (the floating `@stable` pin can break `main` without warning), artifact upload, and `cargo audit`/`cargo deny`.

---

## 9. Documentation & repo hygiene

README is accurate and unusually honest (verified vs. unverified, unapplied features listed). Gaps: no `CHANGELOG`, no `SECURITY.md` (reporting channel, supported versions), no `CONTRIBUTING`, no architecture diagram beyond prose, no documented payload-format spec outside code comments (the footer/archive layout in `payload.rs` docs is good — promote it to `docs/payload-format.md`), demo placeholder not labelled as such (L13), `version` command undocumented (L7). `.gitignore` should cover build outputs in the repo root (L8). Single-commit history suggests the repo was squashed/imported — consider preserving real history going forward for bisectability.

---

## 10. Prioritised roadmap

### Status of this roadmap

P0 items 1–9 are implemented in this branch (H1–H10), and the CI half of item 10 is in place (fmt, clippy, tests, Windows release + smoke test). The **human** half of item 10 — running the wizard and GUI on a real Windows 11 host — is still outstanding, and it is the one thing no amount of CI here can substitute for. P1 is where most of the remaining work sits (M5 streaming, M7 cancel confirmation + logging, M9 worker thread, M11 folder Browse, M18 manifest/version/icon); the L-items are mostly untouched.

### P0 — before any end-user release
1. **H1** Confine uninstall deletions to `install_dir`; validate journal on load.
2. **H2** Cap decompression (`archive_len` sanity + `take` limit); regression tests for lying footer + bomb.
3. **H3** Hard-error on present-but-invalid payload; demo mode only for genuinely unpacked stubs.
4. **H4** Save journal before/during install; implement or remove `OverwriteFile` backups; fix README claims to match.
5. **H5** Fix directory rollback order (deepest-first); report leftover dirs.
6. **H6** OS-independent sanitisation (reject `:`, UNC, ADS, reserved names); symlink-safe extract; symlink-aware pack with cycle detection.
7. **H7** Error (don't truncate) on oversize paths/files/counts in archive + ZIP writers.
8. **H8** Warn/fail-fast on ignored settings; decide `DualMode` semantics; implement or remove unapplied schema fields.
9. **H9/H10** Sanitise version in GUI; validate `main_exe` as bare filename + presence in payload.
10. **Windows verification:** run stub + GUI on a real Windows 11 host (wizard, typewriter, shortcut, Run entry, ARP, install→uninstall round-trip, per-machine with elevation). Record results in README.

### P1 — next slice
M1 (deny unknown fields), M2 (full validation), M4 (pack-time checks), M5 (streaming / single decode), M6 (elevation), M7 (confirmations + log), M8 (keyboard nav), M9 (worker-thread install), M10/M11 (real license page, dir validation + Browse), M13/M14 (shared helpers, CLI/GUI parity), M15–M18 (journal durability, association restore, tray-arg consistency, manifest/version/icon).

### P2 — polish
L1–L17, CI hardening (clippy/fmt/Linux/`--locked`/cache/artifacts/audit/Dependabot), `CHANGELOG`/`SECURITY.md`/`docs/payload-format.md`, `.gitignore` expansion, test-gap coverage (L12), demo placeholder labelling.

---

## Appendix A — Dependency inventory (`Cargo.lock`)

48 packages. Direct: `serde 1.0.229`, `serde_json 1.0.154`, `toml 0.8.23`, `sha2 0.10.9`, `zstd 0.13.5`, `windows-sys 0.61.2` (Windows only). Transitive highlights: `zstd-sys 2.1.3+zstd.1.5.7` (C library — track for CVEs), `toml_edit 0.22.27`, `winnow 0.7.15`, `zmij 1.0.23`. No audit tool was available in this sandbox; run `cargo audit` and `cargo deny check licenses bans` in CI (L10).

## Appendix B — Audit checklist (all items checked)

- [x] All Rust sources read (core, CLI, stub, GUI, tests)
- [x] All manifests read (workspace + 4 crates, `.cargo/config.toml`, CI, `.gitignore`, `LICENSE`, `README`, demo TOML)
- [x] `grep` sweeps: `unwrap`/`expect`/`panic`/`todo`, `unsafe` counts, network/process/env, `deny_unknown_fields`, symlinks, elevation, confirmations, `as`-cast truncations, `with_capacity` on untrusted input, `join` on user input
- [x] Cross-crate consistency (sanitisers, validation, path resolution, filenames, scope handling)
- [x] Schema-vs-stub feature matrix (which fields the stub actually applies)
- [x] Journal lifecycle (record → save → rollback → uninstall) traced end-to-end
- [x] Payload format traced (pack → footer → verify → extract) including failure modes
- [x] `cargo test` / `cargo clippy` / `cargo fmt --check` — **run in CI on this branch** (see the `linux` and `windows-msvc` jobs in `.github/workflows/ci.yml`); the fixes were iterated until both jobs went green
- [ ] Windows runtime verification — **still not run** (no Windows host; the README says so, and CI only compiles/lints those crates)
- [ ] `cargo audit` / advisory check — **not run** (no toolchain; see L10)

*End of audit.*

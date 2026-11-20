# Windows verification runbook

`AUDIT.md` §10 item 10 is the one roadmap item CI cannot close: the wizard
(`rustpacker-stub`) and the Builder Studio (`rustpacker-gui`) are compiled,
linted and packaged in CI but **never run**. This runbook turns "try it on
Windows" into a repeatable pass whose results can be recorded.

## Before you start

- A Windows 11 x64 machine (real hardware or a VM), build ≥ 22000,
  with a standard user account *and* an administrator account available.
- Get the binaries: either download the release assets, or build them:
  `cargo build --release --target x86_64-pc-windows-msvc --workspace`.
- Make a test payload: a folder containing **one real executable** (the
  freshly built `rustpacker-cli.exe` works well — it runs and exits
  cleanly) plus a text file. The `examples/demo` payload is a text
  placeholder and cannot be launched, so don't use it for launch checks.
- Pack a `Setup.exe` from that payload with the CLI (`pack --portable`)
  and note the SHA-256 from `SHA256SUMS.txt`.

Work through the sections in order. For each check, write **Pass**,
**Fail** (plus what happened — attach `install.log` for install failures)
or **n/a** in the Result column.

## A. Wizard flow and keyboard

| # | Check | Expected | Result |
|---|-------|----------|--------|
| A1 | Launch `Setup.exe` | Wizard opens; icon in taskbar/Alt-Tab is the RustPacker icon, not the default | |
| A2 | Welcome page | Text appears with the typewriter effect; clicking (or Space/Enter) completes the text instantly | |
| A3 | Press Space/Enter again with the text complete | Advances to the next page — the wizard can be completed keyboard-only | |
| A4 | License page | Shows the actual license text from the project (not a file path), scrollable | |
| A5 | Directory page | Default folder is sensible; **Browse...** opens the folder picker; cancelling the picker keeps the typed path | |
| A6 | Type an invalid folder (`C:\Windows\System32`, `C:`, a path with `..`) and continue | A clear validation message names the problem; the install does not start | |
| A7 | Installing page | Window keeps repainting; progress moves; status names the file being extracted; Cancel stays clickable | |
| A8 | Click Cancel mid-install | Install stops, a message says changes were rolled back, and the install folder contains no half-installed app | |

## B. Finish page and first run

| # | Check | Expected | Result |
|---|-------|----------|--------|
| B1 | Finish page | Exactly the configured checkboxes appear; tray sub-options are indented and disable when their parent is unticked | |
| B2 | Finish with "Create Desktop shortcut" ticked | A working shortcut with the app icon appears on the Desktop | |
| B3 | Finish with "Launch after setup exits" ticked | The app starts on Finish (with the tray argument if that sub-option was ticked — verify via the app's behaviour/args) | |
| B4 | Finish with "Launch at Windows login" ticked | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` gains a quoted `"path" [--tray]` value for the app; signing out/in starts it | |
| B5 | Click Cancel on the Finish page after a successful install | Asked whether to keep the app; **No** uninstalls it, **Yes** leaves it | |

## C. What the install left behind

| # | Check | Expected | Result |
|---|-------|----------|--------|
| C1 | Install folder contents | App files, `Uninstall.exe`, `install.journal.json`, `install.log` | |
| C2 | `Uninstall.exe` size | Roughly the stub size — clearly smaller than `Setup.exe` (payload stripped) | |
| C3 | `install.log` | Header line, one `+Nms` line per step, ends with `---- finished: installed ----`; readable in Notepad (CRLF) | |
| C4 | Apps & Features | The app is listed with publisher, version and icon; "Uninstall" works from there | |
| C5 | Start Menu / custom shortcuts, `[[registry]]`, `[[file_associations]]`, PATH (per the test project) | Each present as configured; association opens the app | |
| C6 | `install.journal.json` | Every created file/key is recorded; hand-editing a path to outside the install folder makes uninstall **refuse** rather than delete | |

## D. Uninstall

| # | Check | Expected | Result |
|---|-------|----------|--------|
| D1 | Run `Uninstall.exe` | Confirmation prompt first (default **No**); cancelling changes nothing | |
| D2 | Confirm | Files, shortcuts, registry values, Run entry and Apps & Features entry are gone; folders the user added files to survive | |
| D3 | Run `Uninstall.exe --silent` from a terminal | No UI at all; same cleanup as D2 | |
| D4 | Reinstall over an existing install, then uninstall | No duplicates, no errors; cleanup still complete | |

## E. Scope and elevation

| # | Check | Expected | Result |
|---|-------|----------|--------|
| E1 | Per-machine project from a standard account | Early elevation prompt / UAC re-launch — not a mid-install registry error | |
| E2 | Per-machine install as admin | Installs under `%ProgramFiles%`, ARP entry machine-wide | |
| E3 | Cancel the UAC prompt | Installer stops cleanly with an explanation | |

## F. Builder Studio

| # | Check | Expected | Result |
|---|-------|----------|--------|
| F1 | Launch `rustpacker-gui.exe` | Studio opens; no console window; icon present | |
| F2 | Load the test project; toggle tray support and the hide-options | Preview checkboxes keep your earlier ticks; newly appearing options start at their defaults | |
| F3 | Tick "Also build portable .zip" and Build | `Setup.exe`, `<App>-v<version>-Portable.zip` and a `SHA256SUMS.txt` covering **both** appear in the output folder; status reports both hashes | |
| F4 | Unzip the portable build | Payload files plus `rustpacker-portable.txt`; the exe runs from the unzipped folder | |
| F5 | Build with an app name containing `\` or `:` (e.g. `A/B`) | Output names are sanitised identically to the CLI's — no stray subfolders | |
| F6 | Switch between editor pages (Application, Install settings, Shortcuts, …) with unsaved edits | No edit is lost; each page shows its own fields and the centre title/hint updates | |
| F7 | Add, edit and remove a shortcut, a registry entry, a file association, a prerequisite and a process check; save and reload the project | All entries survive the round-trip in the project file, and Validate flags a deliberately broken entry | |
| F8 | Collapse navigation groups, hide the navigation and preview panels, resize the window | Layout stays usable down to the minimum size; nothing overlaps or disappears | |
| F9 | Use the menu bar (File → New/Open/Save, Build → Validate/Build, Help → About) | Same behaviour as the toolbar buttons; About shows the Studio version | |

## G. Failure honesty

| # | Check | Expected | Result |
|---|-------|----------|--------|
| G1 | Rename `Setup.exe`'s payload (append bytes) and run | Refuses with an explanation; no demo/placeholder install | |
| G2 | Install to a nearly full / read-only location | Clear error, rollback, and `install.log` shows where it stopped | |
| G3 | Studio: point the stub path at a text file and Build | "not a Windows executable" error, not a broken `Setup.exe` | |

## Recording the results

When a full pass is done, copy the outcome into the **Status** section of
`README.md` (replace the "Unverified" paragraph with what was run, the
Windows build, and the date) and close AUDIT §10 item 10. File failures as
issues with `install.log` attached — that file existing and being useful
is itself check C3.

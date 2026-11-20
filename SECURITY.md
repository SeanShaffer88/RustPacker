# Security policy

## Supported versions

RustPacker is pre-1.0. Security fixes land on `main` and in the next tagged
release; only the latest release is supported.

| Version | Supported |
|---|---|
| 0.1.x | yes |
| < 0.1 | no |

## Reporting a vulnerability

Please report privately through GitHub's
[Security Advisories](https://github.com/SeanShaffer88/RustPacker/security/advisories/new)
("Report a vulnerability" on the Security tab) rather than in a public issue.

Please include what you did, what happened, and what you expected — plus the
installer or project file if you can share it, since the interesting bugs in
this project tend to live in the payload format and the Win32 surface.

You can expect an acknowledgement within a few days. This is a
single-maintainer project, so please be patient with fixes; a confirmed
issue will be disclosed in the release notes once a fix ships.

## What is in scope

RustPacker builds installers, so its threat model is "an attacker hands a
user a `Setup.exe`, or hands a developer a project file / payload directory".

In scope:

- **Payload and archive parsing** — a crafted `Setup.exe` that makes
  `verify`, `list`, `extract`, or the stub crash, hang, exhaust memory, or
  write outside the destination. The format is documented in
  `AUDIT.md` §3 and every field is length-checked; break that.
- **Path handling** — anything that escapes the install directory, follows a
  pre-existing symlink or junction, or writes through a Windows
  drive/UNC/ADS/reserved-device name.
- **Uninstall safety** — a tampered `install.journal.json` that makes
  uninstall delete or modify files it should not.
- **Elevation** — a path where a per-user install can be made to perform a
  per-machine action, or where the UAC re-launch can be spoofed.
- **Registry and COM** — command strings, `Run` values, file-association
  prog-ids, and shortcut targets that permit argument or quote injection.
- **Build-time code execution** — anything that makes building a project
  file run code unexpectedly.

Out of scope:

- Anything that requires an attacker to already run code as the user.
- Denial of service through enormous *legitimate* payloads (a project author
  may pack a large app; memory use is bounded per file, not per total).
- The absence of code signing on released binaries, and SmartScreen
  warnings that follow from it. This is a known, documented limitation —
  see the "Known limitations" section of `CHANGELOG.md`.
- Vulnerabilities in the Windows APIs RustPacker calls, unless RustPacker
  uses them unsafely.

## Design decisions worth knowing

- Installers are **offline and telemetry-free**; nothing is fetched at
  install time. There is no update channel to attack.
- A packed installer whose payload is corrupt, or whose embedded project
  file is invalid, **refuses to run** rather than falling back to anything.
- `pack` requires the stub to be a real PE image, and refuses a payload that
  does not contain `app.main_exe`.
- The installer requests `asInvoker` in its manifest and elevates only when
  the project asks for a per-machine install.

## Known unverified surface

The wizard and Builder Studio are compiled, linted and tested in CI but have
**never been run on a real Windows host** by the maintainer's automation.
`AUDIT.md` records, finding by finding, which behaviour is runtime-verified
and which is inspection-only. If you have a Windows 11 machine, running the
install → use → uninstall round-trip and reporting what breaks is genuinely
useful security work.

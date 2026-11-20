//! `rustpacker-cli` — command-line packager for RustPacker.
//!
//! Thin surface over `rustpacker-core`: it reads a project file, checks
//! it, packs a payload onto a stub executable, and can verify, list, and
//! extract the resulting `Setup.exe`. Argument parsing is hand-rolled
//! (standard library only); `--json` selects machine-readable output.

use rustpacker_core::payload::{sha256_hex, sha256sums_txt, write_stored_zip_streaming, ZipSource};
use rustpacker_core::{
    extract_payload, find_payload, pack_directory, read_manifest, safe_file_name, PayloadManifest,
    ProjectConfig,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_VALIDATION: i32 = 3;
const EXIT_IO: i32 = 4;

const HELP: &str = r#"RustPacker CLI - pack a project into a Windows Setup.exe.

Usage:
  rustpacker-cli [--json] <command> [options]

Commands:
  init [--out <file>] [--force]
      Write an example project file (default: rustpacker.toml).
  validate <project.toml>
      Check a project file and list every problem found.
  pack <project.toml> --stub <stub.exe> [--out-dir <dir>] [--portable]
      Build <App>-v<version>-Setup.exe plus SHA256SUMS.txt; with
      --portable also build <App>-v<version>-Portable.zip.
  verify <Setup.exe>
      Check the payload checksums and list the packed files.
  list <Setup.exe>
      List the files packed in a Setup.exe.
  extract <Setup.exe> --out <dir> [--force]
      Extract the packed files to a directory (refuses a non-empty
      directory unless --force is given).
  schema
      Print the example project file (TOML) to standard output.
  version
      Print the CLI version.
  help
      Show this help.

Options:
  --json
      Print machine-readable JSON for init, validate, pack, verify,
      list, and extract. May appear before or after the command.

Exit codes:
  0  success
  2  usage error
  3  project validation failed
  4  input/output or payload error

Examples:
  rustpacker-cli init
  rustpacker-cli validate rustpacker.toml
  rustpacker-cli pack rustpacker.toml --stub stub.exe --out-dir dist --portable
  rustpacker-cli --json verify dist/TestApp-v1.2.3-Setup.exe
  rustpacker-cli extract dist/TestApp-v1.2.3-Setup.exe --out unpacked
"#;

// ---------------------------------------------------------------------------
// Result plumbing

struct CmdOk {
    human: String,
    json: Option<String>,
}

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

struct Fail {
    code: i32,
    command: Option<String>,
    errors: Vec<String>,
}

impl Fail {
    fn usage(command: Option<&str>, message: &str) -> Self {
        Self {
            code: EXIT_USAGE,
            command: command.map(str::to_string),
            errors: vec![message.to_string()],
        }
    }

    fn validation(command: &str, errors: Vec<String>) -> Self {
        Self {
            code: EXIT_VALIDATION,
            command: Some(command.to_string()),
            errors,
        }
    }

    fn io(command: &str, message: String) -> Self {
        Self {
            code: EXIT_IO,
            command: Some(command.to_string()),
            errors: vec![message],
        }
    }
}

fn error_text(fail: &Fail) -> String {
    if fail.errors.len() == 1 {
        format!("error: {}\n", fail.errors[0])
    } else {
        let mut out = String::from("error:\n");
        for e in &fail.errors {
            out.push_str(&format!("  - {e}\n"));
        }
        out
    }
}

fn error_json(fail: &Fail) -> String {
    let command = match &fail.command {
        Some(c) => json_str(c),
        None => "null".to_string(),
    };
    format!(
        "{{\"ok\":false,\"command\":{command},\"errors\":{}}}",
        json_string_array(&fail.errors)
    )
}

fn run(args: &[String], json: bool) -> Output {
    match dispatch(args) {
        Ok(ok) => {
            let stdout = match (json, ok.json) {
                (true, Some(j)) => format!("{j}\n"),
                _ => ok.human,
            };
            Output {
                code: EXIT_OK,
                stdout,
                stderr: String::new(),
            }
        }
        Err(fail) => {
            if json {
                Output {
                    code: fail.code,
                    stdout: format!("{}\n", error_json(&fail)),
                    stderr: String::new(),
                }
            } else {
                Output {
                    code: fail.code,
                    stdout: String::new(),
                    stderr: error_text(&fail),
                }
            }
        }
    }
}

fn emit_stdout(text: &str) {
    if text.is_empty() {
        return;
    }
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(text.as_bytes());
    let _ = lock.flush();
}

fn emit_stderr(text: &str) {
    if text.is_empty() {
        return;
    }
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let _ = lock.write_all(text.as_bytes());
    let _ = lock.flush();
}

fn real_main() -> i32 {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut json = false;
    let mut args: Vec<String> = Vec::new();
    // `--json` is a global switch, but everything after a `--` terminator
    // is positional data and must be passed through untouched.
    let mut after_terminator = false;
    for arg in raw {
        if after_terminator {
            args.push(arg);
        } else if arg == "--" {
            after_terminator = true;
        } else if arg == "--json" {
            json = true;
        } else {
            args.push(arg);
        }
    }
    let output = run(&args, json);
    emit_stdout(&output.stdout);
    emit_stderr(&output.stderr);
    output.code
}

fn main() {
    std::process::exit(real_main());
}

// ---------------------------------------------------------------------------
// JSON helpers (small, dependency-free)

fn json_str(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_string_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&json_str(item));
    }
    out.push(']');
    out
}

// ---------------------------------------------------------------------------
// Argument parsing

/// Split `args` into positionals, valued options (`--name value` or
/// `--name=value`), and boolean flags (`--flag`). Names in `valued` /
/// `boolean` are given without the leading dashes.
/// Positionals, `--name value` pairs, and boolean flags, in that order.
type ParsedArgs = (Vec<String>, Vec<(String, String)>, Vec<String>);

fn parse_args(args: &[String], valued: &[&str], boolean: &[&str]) -> Result<ParsedArgs, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut values: Vec<(String, String)> = Vec::new();
    let mut flags: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut only_positional = false;
    while i < args.len() {
        let arg = &args[i];
        if only_positional {
            positional.push(arg.clone());
            i += 1;
            continue;
        }
        if arg == "--" {
            only_positional = true;
            i += 1;
            continue;
        }
        match arg.strip_prefix("--") {
            Some(rest) => {
                let (name, inline_value) = match rest.find('=') {
                    Some(eq) => (&rest[..eq], Some(rest[eq + 1..].to_string())),
                    None => (rest, None),
                };
                if valued.contains(&name) {
                    let value = match inline_value {
                        Some(v) => v,
                        None => {
                            i += 1;
                            match args.get(i) {
                                Some(v) => v.clone(),
                                None => return Err(format!("option --{name} needs a value")),
                            }
                        }
                    };
                    if values.iter().any(|(n, _)| n == name) {
                        return Err(format!("option --{name} was given more than once"));
                    }
                    values.push((name.to_string(), value));
                } else if boolean.contains(&name) {
                    match inline_value {
                        Some(_) => return Err(format!("option --{name} takes no value")),
                        None => flags.push(name.to_string()),
                    }
                } else {
                    return Err(format!("unknown option --{name}"));
                }
            }
            None => {
                if arg.starts_with('-') && arg.len() > 1 {
                    return Err(format!("unknown option {arg}"));
                }
                positional.push(arg.clone());
            }
        }
        i += 1;
    }
    Ok((positional, values, flags))
}

fn opt_value(values: &[(String, String)], name: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for (n, v) in values {
        if n == name {
            found = Some(v.clone());
        }
    }
    found
}

fn has_flag(flags: &[String], name: &str) -> bool {
    flags.iter().any(|f| f == name)
}

// ---------------------------------------------------------------------------
// Path helpers

/// Resolve `raw` (stub or source dir): absolute paths pass through;
/// relative paths prefer the project directory, then the working
/// directory, and finally fall back to the project-directory form so
/// error messages name the most likely intended location.
fn resolve_relative(project_dir: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let in_project = project_dir.join(p);
    if in_project.exists() {
        return in_project;
    }
    let in_cwd = PathBuf::from(raw);
    if in_cwd.exists() {
        return in_cwd;
    }
    in_project
}

fn project_dir_of(project_arg: &str) -> PathBuf {
    match Path::new(project_arg).parent() {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from("."),
    }
}

/// Join an output file name onto an out-dir, keeping `.` out of the
/// displayed path when the out-dir is the working directory.
fn join_out(dir: &Path, name: &str) -> PathBuf {
    if dir.as_os_str().is_empty() || dir == Path::new(".") {
        PathBuf::from(name)
    } else {
        dir.join(name)
    }
}

// ---------------------------------------------------------------------------
// Example project file (init / schema)

/// True when `line` is a top-level `name = []` entry (an empty list).
fn is_empty_array_line(line: &str, name: &str) -> bool {
    let trimmed = line.trim();
    let rest = match trimmed.strip_prefix(name) {
        Some(r) => r,
        None => return false,
    };
    let rest = match rest.trim_start().strip_prefix('=') {
        Some(r) => r,
        None => return false,
    };
    rest.trim() == "[]"
}

/// Cheap structural check for a PE image: `MZ` magic, a plausible
/// `e_lfanew`, and a `PE\0\0` signature at that offset. Full PE parsing
/// is deliberately out of scope — this only stops accidental or obvious
/// non-executables from being packed as a stub.
fn looks_like_pe(bytes: &[u8]) -> bool {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return false;
    }
    let lfanew = u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;
    if lfanew < 0x40 || lfanew.checked_add(4).map_or(true, |end| end > bytes.len()) {
        return false;
    }
    &bytes[lfanew..lfanew + 4] == b"PE\0\0"
}

/// Build the example project TOML from `ProjectConfig::default()`
/// (app name "My App") with structural comments. Empty list entries are
/// dropped from the body: the commented examples at the bottom can be
/// enabled directly without creating a duplicate key.
fn example_toml() -> Result<String, String> {
    let cfg = ProjectConfig::default();
    let raw = cfg.to_toml()?;
    let mut out = String::new();
    out.push_str("# RustPacker project file\n");
    out.push_str("#\n");
    out.push_str("# Edit the values below, then run:\n");
    out.push_str("#   rustpacker-cli validate rustpacker.toml\n");
    out.push_str("#   rustpacker-cli pack rustpacker.toml --stub stub.exe --portable\n");
    out.push_str("#\n");
    out.push_str("# Top-level keys: schema_version (1 - do not change it),\n");
    out.push_str("# typewriter_interval_ms (wizard typing speed, 12 ms by default),\n");
    out.push_str("# path_scope (\"\" to skip, \"user\", or \"machine\"), plus the\n");
    out.push_str("# tables below.\n");
    out.push('\n');
    for line in raw.lines() {
        if [
            "shortcuts",
            "registry",
            "file_associations",
            "prerequisites",
            "process_checks",
        ]
        .iter()
        .any(|name| is_empty_array_line(line, name))
        {
            continue;
        }
        match line {
            "[app]" => {
                out.push_str("# [app] - the application being installed. app_id is the stable\n");
                out.push_str("# identifier used for the uninstall entry; main_exe is the file\n");
                out.push_str("# inside the payload that gets launched.\n");
            }
            "[install]" => {
                out.push_str("# [install] - install scope, default folder, and the payload\n");
                out.push_str("# source directory (relative to this project file).\n");
                out.push_str("# min_windows_build 22000 means Windows 11; 0 disables the check.\n");
            }
            "[tray]" => {
                out.push_str("# [tray] - system-tray behaviour of the installed app.\n");
                out.push_str("# When supports_tray is true, the finish page also offers\n");
                out.push_str("# \"Start minimized to system tray\" (under Launch after setup)\n");
                out.push_str("# and \"Launch at Windows login\". tray_arg is passed when\n");
                out.push_str("# launching minimised - typical values are \"--tray\" or\n");
                out.push_str("# \"--minimized\". startup_tray_arg, when set, is used for\n");
                out.push_str("# the at-login launch instead; empty falls back to tray_arg.\n");
            }
            "[finish]" => {
                out.push_str("# [finish] - which finish-page options are offered. Desktop\n");
                out.push_str("# shortcut and launch-after-setup are always offered unless\n");
                out.push_str("# hidden here; the tray sub-options only appear when\n");
                out.push_str("# tray.supports_tray is true.\n");
            }
            "[theme]" => {
                out.push_str(
                    "# [theme] - One Dark Pro terminal palette for the installer wizard.\n",
                );
            }
            _ => {}
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("\n# Optional extras - remove the leading \"# \" to enable an entry.\n");
    out.push_str("#\n");
    out.push_str("# Shortcuts (location is \"desktop\" or \"start-menu\"; target is\n");
    out.push_str("# relative to the install dir):\n");
    out.push_str("# [[shortcuts]]\n");
    out.push_str("# name = \"My App\"\n");
    out.push_str("# location = \"desktop\"\n");
    out.push_str("# target = \"MyApp.exe\"\n");
    out.push_str("# arguments = \"\"\n");
    out.push_str("#\n");
    out.push_str("# Registry values written at install (root is HKCU or HKLM):\n");
    out.push_str("# [[registry]]\n");
    out.push_str("# root = \"HKCU\"\n");
    out.push_str("# key = \"Software\\\\My App\"\n");
    out.push_str("# name = \"InstallDir\"\n");
    out.push_str("# value = \"\"\n");
    out.push_str("#\n");
    out.push_str("# File associations (extension starts with a dot):\n");
    out.push_str("# [[file_associations]]\n");
    out.push_str("# extension = \".myapp\"\n");
    out.push_str("# prog_id = \"MyApp.Document\"\n");
    out.push_str("# description = \"My App document\"\n");
    out.push_str("# open_with = \"MyApp.exe\"\n");
    out.push_str("#\n");
    out.push_str("# Prerequisites (checked before install starts; give a registry\n");
    out.push_str("# probe, a file probe, or both):\n");
    out.push_str("# [[prerequisites]]\n");
    out.push_str("# name = \"Example Runtime\"\n");
    out.push_str("# registry_probe = \"\"\n");
    out.push_str("# file_probe = \"\"\n");
    out.push_str("# message = \"This app needs the Example Runtime.\"\n");
    out.push_str("#\n");
    out.push_str("# Running-process checks (install waits while these run):\n");
    out.push_str("# [[process_checks]]\n");
    out.push_str("# process_name = \"MyApp.exe\"\n");
    out.push_str("# message = \"Please close My App before installing.\"\n");
    Ok(out)
}

// ---------------------------------------------------------------------------
// Project loading

/// Read, parse, and check a project file. A missing file is an I/O
/// error (exit 4); a malformed file or a failed check is a validation
/// failure (exit 3) listing every problem.
fn load_project_file(path: &str, command: &str) -> Result<ProjectConfig, Fail> {
    let text = fs::read_to_string(path)
        .map_err(|e| Fail::io(command, format!("could not read {path}: {e}")))?;
    // `from_toml_checked` performs migration, normalisation, and full
    // validation, returning every problem at once.
    ProjectConfig::from_toml_checked(&text).map_err(|errors| Fail::validation(command, errors))
}

// ---------------------------------------------------------------------------
// Commands

fn dispatch(args: &[String]) -> Result<CmdOk, Fail> {
    if args.is_empty() {
        return Ok(CmdOk {
            human: HELP.to_string(),
            json: None,
        });
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(CmdOk {
            human: HELP.to_string(),
            json: None,
        });
    }
    match args[0].as_str() {
        "help" => Ok(CmdOk {
            human: HELP.to_string(),
            json: None,
        }),
        "version" | "--version" | "-V" => Ok(CmdOk {
            human: format!("rustpacker-cli {}\n", env!("CARGO_PKG_VERSION")),
            json: None,
        }),
        "init" => cmd_init(&args[1..]),
        "validate" => cmd_validate(&args[1..]),
        "pack" => cmd_pack(&args[1..]),
        "verify" => cmd_verify(&args[1..]),
        "list" => cmd_list(&args[1..]),
        "extract" => cmd_extract(&args[1..]),
        "schema" => cmd_schema(&args[1..]),
        other => Err(Fail::usage(
            None,
            &format!("unknown command '{other}'; run `rustpacker-cli help` for usage"),
        )),
    }
}

fn cmd_init(args: &[String]) -> Result<CmdOk, Fail> {
    let (positional, values, flags) =
        parse_args(args, &["out"], &["force"]).map_err(|m| Fail::usage(Some("init"), &m))?;
    if !positional.is_empty() {
        return Err(Fail::usage(
            Some("init"),
            "init takes no positional arguments",
        ));
    }
    let out = match opt_value(&values, "out") {
        Some(v) => v,
        None => "rustpacker.toml".to_string(),
    };
    let out_path = PathBuf::from(&out);
    if out_path.exists() && !has_flag(&flags, "force") {
        return Err(Fail::io(
            "init",
            format!("{out} already exists (use --force to overwrite it)"),
        ));
    }
    let text = example_toml().map_err(|m| Fail::io("init", m))?;
    match out_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            fs::create_dir_all(parent).map_err(|e| {
                Fail::io(
                    "init",
                    format!("could not create {}: {e}", parent.display()),
                )
            })?;
        }
        _ => {}
    }
    fs::write(&out_path, &text)
        .map_err(|e| Fail::io("init", format!("could not write {out}: {e}")))?;
    let human = format!("Wrote {out}\nEdit it, then run: rustpacker-cli validate {out}\n");
    let json = format!(
        "{{\"ok\":true,\"command\":\"init\",\"path\":{},\"errors\":[]}}",
        json_str(&out)
    );
    Ok(CmdOk {
        human,
        json: Some(json),
    })
}

fn cmd_validate(args: &[String]) -> Result<CmdOk, Fail> {
    let (positional, _values, _flags) =
        parse_args(args, &[], &[]).map_err(|m| Fail::usage(Some("validate"), &m))?;
    if positional.len() != 1 {
        return Err(Fail::usage(
            Some("validate"),
            "usage: rustpacker-cli validate <project.toml>",
        ));
    }
    let path = &positional[0];
    let cfg = load_project_file(path, "validate")?;
    // `validate` is the command people trust before packing, so it also
    // checks the things only the filesystem can answer.
    let mut warnings: Vec<String> = Vec::new();
    let project_dir = project_dir_of(path);
    let source = resolve_relative(&project_dir, &cfg.install.source_dir);
    if !source.is_dir() {
        warnings.push(format!(
            "payload source directory not found: {}",
            source.display()
        ));
    }
    if !cfg.install.license_file.trim().is_empty() {
        let license = resolve_relative(&project_dir, &cfg.install.license_file);
        if !license.is_file() {
            warnings.push(format!("license file not found: {}", license.display()));
        }
    }
    // Keep the "Valid:" prefix stable for scripts; extra filesystem
    // warnings are appended rather than replacing it.
    let mut human = format!("Valid: {path} - {} v{}\n", cfg.app.name, cfg.app.version);
    if !warnings.is_empty() {
        human.push_str("Note: these will stop `pack` until fixed:\n");
        for w in &warnings {
            human.push_str(&format!("  - {w}\n"));
        }
    }
    let json = format!(
        "{{\"ok\":true,\"command\":\"validate\",\"path\":{},\"app\":{},\"version\":{},\"warnings\":{},\"errors\":[]}}",
        json_str(path),
        json_str(&cfg.app.name),
        json_str(&cfg.app.version),
        json_string_array(&warnings)
    );
    Ok(CmdOk {
        human,
        json: Some(json),
    })
}

fn cmd_pack(args: &[String]) -> Result<CmdOk, Fail> {
    let (positional, values, flags) = parse_args(args, &["stub", "out-dir"], &["portable"])
        .map_err(|m| Fail::usage(Some("pack"), &m))?;
    if positional.len() != 1 {
        return Err(Fail::usage(
            Some("pack"),
            "usage: rustpacker-cli pack <project.toml> --stub <stub.exe> [--out-dir <dir>] [--portable]",
        ));
    }
    let project_arg = &positional[0];
    let stub_arg = match opt_value(&values, "stub") {
        Some(v) => v,
        None => return Err(Fail::usage(Some("pack"), "--stub <stub.exe> is required")),
    };
    let out_dir_arg = match opt_value(&values, "out-dir") {
        Some(v) => v,
        None => ".".to_string(),
    };
    let want_portable = has_flag(&flags, "portable");

    let cfg = load_project_file(project_arg, "pack")?;
    let project_dir = project_dir_of(project_arg);

    let stub_path = resolve_relative(&project_dir, &stub_arg);
    if !stub_path.is_file() {
        return Err(Fail::io(
            "pack",
            format!("stub not found: {}", stub_path.display()),
        ));
    }
    let stub_bytes = fs::read(&stub_path).map_err(|e| {
        Fail::io(
            "pack",
            format!("could not read {}: {e}", stub_path.display()),
        )
    })?;

    let source_dir = resolve_relative(&project_dir, &cfg.install.source_dir);
    // Pack-time checks the schema cannot make (it does not see the disk):
    // a missing source directory, a missing license file, and a stub that
    // is not a Windows executable all fail here rather than at install time.
    if !source_dir.is_dir() {
        return Err(Fail::io(
            "pack",
            format!(
                "payload source directory not found: {} (install.source_dir = {:?})",
                source_dir.display(),
                cfg.install.source_dir
            ),
        ));
    }
    // Embed the license text so the wizard's license page shows the real
    // agreement (and keeps working after the source tree is gone).
    let mut cfg = cfg;
    if !cfg.install.license_file.trim().is_empty() {
        let license = resolve_relative(&project_dir, &cfg.install.license_file);
        let text = fs::read_to_string(&license).map_err(|e| {
            Fail::io(
                "pack",
                format!(
                    "could not read license file {}: {e} (it must be UTF-8 text)",
                    license.display()
                ),
            )
        })?;
        if text.trim().is_empty() {
            return Err(Fail::validation(
                "pack",
                vec![format!("license file {} is empty", license.display())],
            ));
        }
        cfg.install.license_text = text;
    } else if !cfg.install.license_text.trim().is_empty() {
        // Text left over from a hand-edited project file is fine, but cap
        // it so a huge blob cannot bloat every installer payload.
        const MAX_LICENSE_BYTES: usize = 256 * 1024;
        if cfg.install.license_text.len() > MAX_LICENSE_BYTES {
            return Err(Fail::validation(
                "pack",
                vec![format!(
                    "install.license_text is {} bytes (max {MAX_LICENSE_BYTES}); \
                     point license_file at a file instead",
                    cfg.install.license_text.len()
                )],
            ));
        }
    }
    // Reject anything that is not a PE image: a text file or a renamed
    // archive as `--stub` would otherwise produce an installer that
    // Windows refuses to launch.
    if !looks_like_pe(&stub_bytes) {
        return Err(Fail::io(
            "pack",
            format!(
                "{} is not a Windows executable (no MZ/PE headers); pass the RustPacker stub",
                stub_path.display()
            ),
        ));
    }
    let config_json = cfg
        .to_json()
        .map_err(|m| Fail::io("pack", format!("could not serialise project: {m}")))?;
    let (setup_bytes, entries) =
        pack_directory(&stub_bytes, &source_dir, &config_json).map_err(|m| Fail::io("pack", m))?;
    // The installer launches app.main_exe; if the payload does not contain
    // it, the build produces an installer that fails at launch time.
    if !entries
        .iter()
        .any(|e| e.path.eq_ignore_ascii_case(cfg.app.main_exe.trim()))
    {
        return Err(Fail::validation(
            "pack",
            vec![format!(
                "app.main_exe '{}' is not in the payload from {}; check install.source_dir",
                cfg.app.main_exe,
                source_dir.display()
            )],
        ));
    }

    let out_dir = PathBuf::from(&out_dir_arg);
    fs::create_dir_all(&out_dir).map_err(|e| {
        Fail::io(
            "pack",
            format!("could not create {}: {e}", out_dir.display()),
        )
    })?;

    let app_stem = safe_file_name(&cfg.app.name);
    let version_stem = safe_file_name(&cfg.app.version);
    let setup_name = format!("{app_stem}-v{version_stem}-Setup.exe");
    let setup_path = join_out(&out_dir, &setup_name);

    // ---- build every output in memory, then write them as one unit --------
    //
    // L4: writing each file as it is produced meant a failure halfway through
    // left a Setup.exe with no SHA256SUMS.txt -- an installer that looks
    // finished and cannot be verified. Both payloads are already built by
    // this point, so the only remaining failure mode is the filesystem, and
    // for that a rollback is exact: delete what this run created and restore
    // what it replaced.
    let mut portable_name: Option<String> = None;
    let mut portable_bytes: Vec<u8> = Vec::new();
    if want_portable {
        // The payload walk already produced the manifest; reuse it instead
        // of walking (and reading) the whole source tree a second time. The
        // ZIP writer streams each file from disk, so only 64 KiB per entry
        // is resident.
        let marker = portable_marker(&cfg, entries.len());
        let zip_paths: Vec<PathBuf> = entries
            .iter()
            .map(|entry| source_dir.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        let mut refs: Vec<(String, ZipSource<'_>)> = Vec::with_capacity(entries.len() + 1);
        for (entry, path) in entries.iter().zip(&zip_paths) {
            refs.push((entry.path.clone(), ZipSource::Path(path)));
        }
        refs.push((
            "rustpacker-portable.txt".to_string(),
            ZipSource::Bytes(marker.as_bytes()),
        ));
        portable_bytes = write_stored_zip_streaming(&refs).map_err(|m| Fail::io("pack", m))?;
        portable_name = Some(format!("{app_stem}-v{version_stem}-Portable.zip"));
    }

    let sums_name = "SHA256SUMS.txt".to_string();
    let mut sum_items: Vec<(&str, &[u8])> = vec![(setup_name.as_str(), setup_bytes.as_slice())];
    if let Some(name) = &portable_name {
        sum_items.push((name.as_str(), portable_bytes.as_slice()));
    }
    let sums_text = sha256sums_txt(&sum_items);
    let sums_path = join_out(&out_dir, &sums_name);

    let portable_path = portable_name.as_ref().map(|n| join_out(&out_dir, n));

    let mut planned: Vec<(&Path, &[u8])> = vec![(&setup_path, &setup_bytes)];
    if let (Some(path), Some(_)) = (&portable_path, &portable_name) {
        planned.push((path, &portable_bytes));
    }
    // Checksums last: they describe the final bytes of the other two.
    planned.push((&sums_path, sums_text.as_bytes()));

    write_outputs(&planned).map_err(|m| Fail::io("pack", m))?;

    let file_count = entries.len();
    let total_bytes: u64 = entries.iter().map(|e| e.size).sum();
    let file_list: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
    let setup_display = setup_path.display().to_string();
    let sums_display = sums_path.display().to_string();

    let mut human = format!(
        "Packed {} v{}\n  Setup: {setup_display} ({file_count} files, {total_bytes} payload bytes)\n",
        cfg.app.name, cfg.app.version
    );
    if let Some(p) = &portable_path {
        human.push_str(&format!("  Portable: {}\n", p.display()));
    }
    human.push_str(&format!("  Checksums: {sums_display}\n"));

    let json = format!(
        "{{\"ok\":true,\"command\":\"pack\",\"app\":{},\"version\":{},\"setup\":{},\"portable\":{},\"sha256sums\":{},\"setup_sha256\":{},\"portable_sha256\":{},\"files\":{},\"file_count\":{},\"total_bytes\":{},\"errors\":[]}}",
        json_str(&cfg.app.name),
        json_str(&cfg.app.version),
        json_str(&setup_display),
        match &portable_path {
            Some(p) => json_str(&p.display().to_string()),
            None => "null".to_string(),
        },
        json_str(&sums_display),
        json_str(&sha256_hex(&setup_bytes)),
        match &portable_path {
            Some(_) => json_str(&sha256_hex(&portable_bytes)),
            None => "null".to_string(),
        },
        json_string_array(&file_list),
        file_count,
        total_bytes
    );
    Ok(CmdOk {
        human,
        json: Some(json),
    })
}

/// Write a set of output files as one unit.
///
/// Each file is written to `<name>.tmp` first and then renamed into place, so
/// a reader never observes a half-written artifact. If any write or rename
/// fails, everything this call created is removed and any replaced file is
/// restored from its backup — `pack` either produces the complete set or
/// changes nothing.
fn write_outputs(files: &[(&Path, &[u8])]) -> Result<(), String> {
    // Back up anything we are about to overwrite, so a rollback can put the
    // previous generation back instead of deleting the user's last good build.
    let mut backups: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    let mut written: Vec<PathBuf> = Vec::new();

    let rollback = |backups: &[(PathBuf, Option<Vec<u8>>)], written: &[PathBuf]| {
        for path in written.iter().rev() {
            let _ = fs::remove_file(path);
        }
        for (path, previous) in backups {
            match previous {
                Some(bytes) => {
                    let _ = fs::write(path, bytes);
                }
                None => {
                    let _ = fs::remove_file(path);
                }
            }
        }
    };

    for (path, bytes) in files {
        let previous = match fs::read(path) {
            Ok(existing) => Some(existing),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                rollback(&backups, &written);
                return Err(format!("could not inspect {}: {e}", path.display()));
            }
        };
        backups.push((path.to_path_buf(), previous));

        // `Setup.exe` -> `Setup.exe.tmp` (with_extension would replace .exe).
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        // Write, then rename: a crash mid-write leaves the .tmp, never a
        // truncated Setup.exe.
        if let Err(e) = fs::write(&tmp, bytes) {
            let _ = fs::remove_file(&tmp);
            rollback(&backups, &written);
            return Err(format!("could not write {}: {e}", tmp.display()));
        }
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            rollback(&backups, &written);
            return Err(format!("could not finish writing {}: {e}", path.display()));
        }
        written.push(path.to_path_buf());
    }
    Ok(())
}

fn portable_marker(cfg: &ProjectConfig, file_count: usize) -> String {
    format!(
        "RustPacker portable export\nApp: {}\nVersion: {}\nPublisher: {}\nFiles: {}\n\nRun {} to start the app.\n",
        cfg.app.name, cfg.app.version, cfg.app.publisher, file_count, cfg.app.main_exe
    )
}

fn cmd_read_setup(args: &[String], command: &str) -> Result<(String, Vec<u8>), Fail> {
    let (positional, _values, _flags) =
        parse_args(args, &[], &[]).map_err(|m| Fail::usage(Some(command), &m))?;
    if positional.len() != 1 {
        return Err(Fail::usage(
            Some(command),
            &format!("usage: rustpacker-cli {command} <Setup.exe>"),
        ));
    }
    let path = positional[0].clone();
    let bytes =
        fs::read(&path).map_err(|e| Fail::io(command, format!("could not read {path}: {e}")))?;
    Ok((path, bytes))
}

fn cmd_verify(args: &[String]) -> Result<CmdOk, Fail> {
    let (path, bytes) = cmd_read_setup(args, "verify")?;
    let manifest = read_manifest(&bytes).map_err(|m| Fail::io("verify", m))?;
    // `verify` is the "will this installer actually work?" command, so it
    // also checks what the stub checks at startup: an embedded project
    // config that parses and validates. `list` stays lenient.
    let mut problems: Vec<String> = Vec::new();
    match &manifest.config_json {
        None => problems.push(
            "the payload has no embedded project config (the installer cannot run)".to_string(),
        ),
        Some(json) => match ProjectConfig::from_json(json) {
            Ok(cfg) => problems.extend(cfg.validate()),
            Err(e) => problems.push(format!("the embedded project config is invalid: {e}")),
        },
    }
    if !problems.is_empty() {
        return Err(Fail::validation("verify", problems));
    }
    Ok(setup_report("verify", &path, &bytes, &manifest))
}

fn cmd_list(args: &[String]) -> Result<CmdOk, Fail> {
    let (path, bytes) = cmd_read_setup(args, "list")?;
    let manifest = read_manifest(&bytes).map_err(|m| Fail::io("list", m))?;
    Ok(setup_report("list", &path, &bytes, &manifest))
}

fn setup_report(command: &str, path: &str, bytes: &[u8], manifest: &PayloadManifest) -> CmdOk {
    let payload_sha = match find_payload(bytes) {
        Ok(info) => info.payload_sha256_hex,
        Err(_) => String::new(),
    };
    let (app, version) = match &manifest.config_json {
        Some(text) => match ProjectConfig::from_json(text) {
            Ok(cfg) => (Some(cfg.app.name), Some(cfg.app.version)),
            Err(_) => (None, None),
        },
        None => (None, None),
    };
    let total_bytes: u64 = manifest.files.iter().map(|f| f.size).sum();
    let file_count = manifest.files.len();

    let mut human = String::new();
    if command == "verify" {
        human.push_str(&format!("Verified: {path}\n"));
        if let (Some(a), Some(v)) = (&app, &version) {
            human.push_str(&format!("App: {a} v{v}\n"));
        }
        if !payload_sha.is_empty() {
            human.push_str(&format!("Payload SHA-256: {payload_sha}\n"));
        }
        human.push_str(&format!("{file_count} files, {total_bytes} bytes:\n"));
    } else {
        human.push_str(&format!(
            "{path}: {file_count} files, {total_bytes} bytes\n"
        ));
    }
    for f in &manifest.files {
        human.push_str(&format!("  {:>10}  {}  {}\n", f.size, f.path, f.sha256_hex));
    }

    let mut files_json = String::from("[");
    for (i, f) in manifest.files.iter().enumerate() {
        if i > 0 {
            files_json.push(',');
        }
        files_json.push_str(&format!(
            "{{\"path\":{},\"size\":{},\"sha256\":{}}}",
            json_str(&f.path),
            f.size,
            json_str(&f.sha256_hex)
        ));
    }
    files_json.push(']');
    let json = format!(
        "{{\"ok\":true,\"command\":{},\"path\":{},\"app\":{},\"version\":{},\"payload_sha256\":{},\"files\":{},\"file_count\":{},\"total_bytes\":{},\"errors\":[]}}",
        json_str(command),
        json_str(path),
        match &app {
            Some(a) => json_str(a),
            None => "null".to_string(),
        },
        match &version {
            Some(v) => json_str(v),
            None => "null".to_string(),
        },
        json_str(&payload_sha),
        files_json,
        file_count,
        total_bytes
    );
    CmdOk {
        human,
        json: Some(json),
    }
}

fn cmd_extract(args: &[String]) -> Result<CmdOk, Fail> {
    let (positional, values, _flags) =
        parse_args(args, &["out"], &["force"]).map_err(|m| Fail::usage(Some("extract"), &m))?;
    if positional.len() != 1 {
        return Err(Fail::usage(
            Some("extract"),
            "usage: rustpacker-cli extract <Setup.exe> --out <dir> [--force]",
        ));
    }
    let path = positional[0].clone();
    let out = match opt_value(&values, "out") {
        Some(v) => v,
        None => return Err(Fail::usage(Some("extract"), "--out <dir> is required")),
    };
    let bytes =
        fs::read(&path).map_err(|e| Fail::io("extract", format!("could not read {path}: {e}")))?;
    let out_dir = PathBuf::from(&out);
    // Extracting into a populated folder can silently overwrite files the
    // user cares about; `--force` makes that explicit.
    if out_dir.exists() {
        let occupied = fs::read_dir(&out_dir)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false);
        if occupied && !has_flag(&_flags, "force") {
            return Err(Fail::io(
                "extract",
                format!(
                    "{out} is not empty (pass --force to extract into it anyway; \
                     files with the same names will be overwritten)"
                ),
            ));
        }
    }
    fs::create_dir_all(&out_dir)
        .map_err(|e| Fail::io("extract", format!("could not create {out}: {e}")))?;
    let extracted = extract_payload(&bytes, &out_dir).map_err(|m| Fail::io("extract", m))?;
    let file_count = extracted.len();
    let mut human = format!("Extracted {file_count} files to {out}:\n");
    for f in &extracted {
        human.push_str(&format!("  {f}\n"));
    }
    let json = format!(
        "{{\"ok\":true,\"command\":\"extract\",\"path\":{},\"out\":{},\"files\":{},\"file_count\":{},\"errors\":[]}}",
        json_str(&path),
        json_str(&out),
        json_string_array(&extracted),
        file_count
    );
    Ok(CmdOk {
        human,
        json: Some(json),
    })
}

fn cmd_schema(args: &[String]) -> Result<CmdOk, Fail> {
    let (positional, _values, _flags) =
        parse_args(args, &[], &[]).map_err(|m| Fail::usage(Some("schema"), &m))?;
    if !positional.is_empty() {
        return Err(Fail::usage(Some("schema"), "schema takes no arguments"));
    }
    let text = example_toml().map_err(|m| Fail::io("schema", m))?;
    Ok(CmdOk {
        human: text,
        json: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_flags_and_values() {
        let args = vec![
            "p.toml".to_string(),
            "--stub".to_string(),
            "s.exe".to_string(),
            "--portable".to_string(),
        ];
        let (positional, values, flags) =
            parse_args(&args, &["stub"], &["portable"]).expect("parse");
        assert_eq!(positional, vec!["p.toml".to_string()]);
        assert_eq!(opt_value(&values, "stub").as_deref(), Some("s.exe"));
        assert!(has_flag(&flags, "portable"));
    }

    #[test]
    fn unknown_option_is_a_usage_error() {
        let args = vec!["--nope".to_string()];
        assert!(parse_args(&args, &[], &[]).is_err());
    }

    #[test]
    fn missing_value_is_a_usage_error() {
        let args = vec!["--out-dir".to_string()];
        assert!(parse_args(&args, &["out-dir"], &[]).is_err());
    }

    #[test]
    fn json_escaping_handles_quotes_and_controls() {
        assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_str("line\nbreak"), "\"line\\nbreak\"");
        assert_eq!(json_string_array(&["a".to_string()]), "[\"a\"]");
    }

    #[test]
    fn output_names_are_windows_safe() {
        // The shared sanitiser must strip path separators from a version.
        assert_eq!(safe_file_name("1.0/../evil"), "1.0-..-evil");
        assert_eq!(safe_file_name("CON"), "_CON");
    }

    #[test]
    fn write_outputs_writes_everything() {
        let dir = std::env::temp_dir().join(format!("rustpacker-wo-{}-a", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let a = dir.join("A-Setup.exe");
        let b = dir.join("SHA256SUMS.txt");
        write_outputs(&[(&a, b"setup"), (&b, b"sums")]).expect("write");
        assert_eq!(std::fs::read(&a).expect("read a"), b"setup");
        assert_eq!(std::fs::read(&b).expect("read b"), b"sums");
        // No .tmp files are left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_outputs_replaces_an_earlier_build() {
        let dir = std::env::temp_dir().join(format!("rustpacker-wo-{}-b", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let a = dir.join("A-Setup.exe");
        std::fs::write(&a, b"old build").expect("seed");
        write_outputs(&[(&a, b"new build")]).expect("write");
        assert_eq!(std::fs::read(&a).expect("read"), b"new build");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_outputs_rolls_back_when_a_later_file_fails() {
        let dir = std::env::temp_dir().join(format!("rustpacker-wo-{}-c", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let good = dir.join("A-Setup.exe");
        std::fs::write(&good, b"previous generation").expect("seed");
        // A directory where a file is expected: the write must fail, and the
        // earlier file must be restored to the bytes it had before.
        let blocked = dir.join("SHA256SUMS.txt");
        std::fs::create_dir_all(&blocked).expect("mkdir blocker");
        let err = write_outputs(&[(&good, b"new generation"), (&blocked, b"sums")])
            .expect_err("must fail");
        assert!(err.contains("SHA256SUMS"), "{err}");
        assert_eq!(
            std::fs::read(&good).expect("read"),
            b"previous generation",
            "the successful write must be rolled back"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn example_toml_round_trips_and_validates() {
        // The template printed by `init`/`schema` and the serialiser used
        // by the GUI must both produce a project that parses and passes
        // validation (deny_unknown_fields would catch a stale template).
        let text = example_toml().expect("template");
        let cfg = ProjectConfig::from_toml_checked(&text)
            .unwrap_or_else(|errs| panic!("template does not validate: {errs:?}"));
        assert!(cfg.validate().is_empty());
        let json = ProjectConfig::default().to_json().expect("json");
        ProjectConfig::from_json(&json).expect("default JSON must parse back");
    }
}

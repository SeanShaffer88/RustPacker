//! End-to-end tests for the `rustpacker-cli` binary: init -> validate,
//! pack -> verify/list/extract round-trips through the real executable.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rustpacker-cli")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "rustpacker-cli-test-{}-{}-{}",
            std::process::id(),
            name,
            n
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn join(&self, rel: &str) -> PathBuf {
        self.path.join(rel)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(cli_bin())
        .args(args)
        .output()
        .expect("run rustpacker-cli")
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(cli_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run rustpacker-cli")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json_value(output: &Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("stdout should be JSON")
}

/// A minimal, *plausible* PE image: `MZ` stub, e_lfanew at 0x3C, and a
/// `PE\0\0` signature there. `pack` now rejects anything that is not a
/// Windows executable, so tests need a header that passes that check.
fn stub_bytes() -> Vec<u8> {
    let mut v = vec![0u8; 512];
    v[0] = b'M';
    v[1] = b'Z';
    // e_lfanew (u32 LE at 0x3C) -> 0x40
    v[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    v[0x40..0x44].copy_from_slice(b"PE\0\0");
    v
}

const PROJECT_TOML: &str = r##"schema_version = 1
typewriter_interval_ms = 12
path_scope = ""

[app]
name = "TestApp"
version = "1.2.3"
publisher = "Sean Shaffer"
app_id = "TestApp"
main_exe = "TestApp.exe"
description = "Test app"

[install]
scope = "per-user"
default_dir = "{per-user-programs}"
source_dir = "payload"
license_file = ""
require_admin = false
min_windows_build = 22000

[tray]
supports_tray = true
tray_arg = "--tray"
startup_tray_arg = ""

[finish]
hide_desktop_shortcut = false
hide_launch_after_setup = false
desktop_shortcut_default = true
launch_after_setup_default = true
launch_tray_default = false
launch_at_login_default = false
login_tray_default = true

[theme]
background_hex = "#282C34"
surface_hex = "#21252B"
elevated_surface_hex = "#2C313A"
border_hex = "#181A1F"
accent_hex = "#E06C75"
success_hex = "#98C379"
warning_hex = "#E5C07B"
info_hex = "#61AFEF"
text_hex = "#ABB2BF"
muted_hex = "#5C6370"
cursor_hex = "#528BFF"
"##;

/// A small fixture project: payload dir with two files, a stub, and the
/// project file. Returns (project path, stub path).
fn write_fixture(dir: &TempDir) -> (PathBuf, PathBuf) {
    let payload = dir.join("payload");
    fs::create_dir_all(payload.join("docs")).expect("mkdir payload");
    fs::write(payload.join("TestApp.exe"), b"fake-exe-bytes").expect("write exe");
    fs::write(payload.join("docs").join("readme.txt"), b"hello rustpacker").expect("write readme");
    let stub = dir.join("stub.exe");
    fs::write(&stub, stub_bytes()).expect("write stub");
    let project = dir.join("project.toml");
    fs::write(&project, PROJECT_TOML).expect("write project");
    (project, stub)
}

#[test]
fn help_and_no_args_show_usage() {
    let out = run(&["help"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("pack"));
    let out = run(&[]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("Usage"));
    let out = run(&["--help"]);
    assert!(out.status.success());
}

#[test]
fn init_creates_file_that_validate_accepts() {
    let dir = TempDir::new("init");
    let project = dir.join("project.toml");
    let project_s = project.to_string_lossy().into_owned();

    let out = run(&["init", "--out", &project_s]);
    assert!(out.status.success(), "init failed: {}", stderr(&out));
    let text = fs::read_to_string(&project).expect("read project");
    assert!(text.contains("My App"), "example names the default app");
    assert!(text.contains("[tray]"), "example has a tray section");
    assert!(
        text.contains("supports_tray"),
        "tray fields are real fields"
    );

    // Refuses to overwrite without --force, succeeds with it.
    let out = run(&["init", "--out", &project_s]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "overwrite without --force should exit 4"
    );
    let out = run(&["init", "--out", &project_s, "--force"]);
    assert!(
        out.status.success(),
        "init --force failed: {}",
        stderr(&out)
    );

    let out = run(&["validate", &project_s]);
    assert!(out.status.success(), "validate failed: {}", stderr(&out));
    assert!(
        stdout(&out).starts_with("Valid:"),
        "validate output: {}",
        stdout(&out)
    );

    // --json after the command is also accepted.
    let out = run(&["validate", &project_s, "--json"]);
    assert!(out.status.success());
    let v = json_value(&out);
    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["command"].as_str(), Some("validate"));
    assert_eq!(v["app"].as_str(), Some("My App"));
    // Filesystem checks are reported as warnings, not silence.
    assert!(v["warnings"].is_array(), "warnings array present: {v}");
}

#[test]
fn validate_reports_missing_source_dir_and_license() {
    let dir = TempDir::new("missing");
    let project = dir.join("p.toml");
    fs::write(
        &project,
        PROJECT_TOML.replace("license_file = \"\"", "license_file = \"LICENSE.txt\""),
    )
    .expect("write project");
    let project_s = project.to_string_lossy().into_owned();
    let out = run(&["validate", &project_s]);
    // Still exit 0: the project file itself is valid, but the human output
    // must name both problems before `pack` fails on them.
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("source directory not found"), "{text}");
    assert!(text.contains("license file not found"), "{text}");
    let v = json_value(&run(&["validate", &project_s, "--json"]));
    assert_eq!(v["warnings"].as_array().map(|a| a.len()), Some(2), "{v}");
}

#[test]
fn unknown_project_field_is_rejected() {
    let dir = TempDir::new("typo");
    let project = dir.join("p.toml");
    fs::write(
        &project,
        PROJECT_TOML.replace("[tray]\nsupports_tray = true", "[tray]\nsuports_tray = true"),
    )
    .expect("write project");
    let project_s = project.to_string_lossy().into_owned();
    let out = run(&["validate", &project_s]);
    assert_eq!(out.status.code(), Some(3), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("suports_tray"), "{}", stderr(&out));
}

#[test]
fn extract_refuses_non_empty_dir_without_force() {
    let dir = TempDir::new("extract-force");
    let (project, stub) = write_fixture(&dir);
    let out_dir = dir.join("out");
    let out = run(&[
        "pack",
        project.to_str().expect("path"),
        "--stub",
        stub.to_str().expect("path"),
        "--out-dir",
        out_dir.to_str().expect("path"),
    ]);
    assert!(out.status.success(), "pack failed: {}", stderr(&out));
    let setup = out_dir.join("TestApp-v1.2.3-Setup.exe");
    let first = dir.join("first");
    fs::create_dir_all(&first).expect("mkdir");
    fs::write(first.join("keep.txt"), b"do not clobber").expect("write");
    let out = run(&[
        "extract",
        setup.to_str().expect("path"),
        "--out",
        first.to_str().expect("path"),
    ]);
    assert_eq!(out.status.code(), Some(4), "stdout: {}", stdout(&out));
    assert!(stderr(&out).contains("not empty"), "{}", stderr(&out));
    // --force allows it.
    let out = run(&[
        "extract",
        setup.to_str().expect("path"),
        "--out",
        first.to_str().expect("path"),
        "--force",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

#[test]
fn schema_prints_valid_toml() {
    let dir = TempDir::new("schema");
    let out = run(&["schema"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("[app]"));
    assert!(text.contains("[tray]"));
    let project = dir.join("schema.toml");
    fs::write(&project, &text).expect("write schema output");
    let project_s = project.to_string_lossy().into_owned();
    let out = run(&["validate", &project_s]);
    assert!(
        out.status.success(),
        "schema output should validate: {}",
        stderr(&out)
    );
}

#[test]
fn pack_verify_list_extract_round_trip() {
    let dir = TempDir::new("pack");
    let (project, stub) = write_fixture(&dir);
    let project_s = project.to_string_lossy().into_owned();
    let stub_s = stub.to_string_lossy().into_owned();
    let out_dir = dir.join("dist");
    let out_dir_s = out_dir.to_string_lossy().into_owned();

    // Run from an unrelated working directory: the payload source dir
    // must resolve relative to the project file, not the cwd.
    let elsewhere = dir.join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("mkdir elsewhere");
    let out = run_in(
        &elsewhere,
        &[
            "pack",
            &project_s,
            "--stub",
            &stub_s,
            "--out-dir",
            &out_dir_s,
            "--portable",
        ],
    );
    assert!(
        out.status.success(),
        "pack failed: {} {}",
        stdout(&out),
        stderr(&out)
    );

    let setup = out_dir.join("TestApp-v1.2.3-Setup.exe");
    let portable = out_dir.join("TestApp-v1.2.3-Portable.zip");
    let sums = out_dir.join("SHA256SUMS.txt");
    assert!(setup.is_file(), "Setup.exe missing");
    assert!(portable.is_file(), "Portable.zip missing");
    assert!(sums.is_file(), "SHA256SUMS.txt missing");
    let sums_text = fs::read_to_string(&sums).expect("read sums");
    assert!(
        sums_text.contains("TestApp-v1.2.3-Setup.exe"),
        "sums cover the Setup.exe"
    );
    assert!(
        sums_text.contains("TestApp-v1.2.3-Portable.zip"),
        "sums cover the portable zip"
    );
    let portable_bytes = fs::read(&portable).expect("read portable");
    assert_eq!(&portable_bytes[0..4], b"PK\x03\x04", "portable is a zip");
    assert!(
        portable_bytes
            .windows(b"rustpacker-portable.txt".len())
            .any(|w| w == b"rustpacker-portable.txt"),
        "portable zip carries the marker entry"
    );

    let setup_s = setup.to_string_lossy().into_owned();

    // verify (human)
    let out = run(&["verify", &setup_s]);
    assert!(out.status.success(), "verify failed: {}", stderr(&out));
    assert!(stdout(&out).contains("TestApp.exe"));
    assert!(stdout(&out).contains("docs/readme.txt"));

    // list (JSON, flag before the command)
    let out = run(&["--json", "list", &setup_s]);
    assert!(out.status.success(), "list failed: {}", stderr(&out));
    let v = json_value(&out);
    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["command"].as_str(), Some("list"));
    assert_eq!(v["file_count"].as_u64(), Some(2));
    assert_eq!(v["app"].as_str(), Some("TestApp"));
    assert_eq!(v["version"].as_str(), Some("1.2.3"));

    // extract
    let extracted_dir = dir.join("extracted");
    let extracted_s = extracted_dir.to_string_lossy().into_owned();
    let out = run(&["extract", &setup_s, "--out", &extracted_s]);
    assert!(out.status.success(), "extract failed: {}", stderr(&out));
    assert_eq!(
        fs::read(extracted_dir.join("TestApp.exe")).expect("read extracted exe"),
        b"fake-exe-bytes"
    );
    assert_eq!(
        fs::read(extracted_dir.join("docs").join("readme.txt")).expect("read extracted readme"),
        b"hello rustpacker"
    );

    // verify rejects a plain stub with no payload: exit 4.
    let out = run(&["verify", &stub_s]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "verify of unpacked file should exit 4"
    );
}

#[test]
fn pack_json_output_is_machine_readable() {
    let dir = TempDir::new("packjson");
    let (project, stub) = write_fixture(&dir);
    let project_s = project.to_string_lossy().into_owned();
    let stub_s = stub.to_string_lossy().into_owned();
    let out_dir = dir.join("dist");
    let out_dir_s = out_dir.to_string_lossy().into_owned();

    let out = run(&[
        "pack",
        "--json",
        &project_s,
        "--stub",
        &stub_s,
        "--out-dir",
        &out_dir_s,
    ]);
    assert!(out.status.success(), "pack --json failed: {}", stderr(&out));
    let v = json_value(&out);
    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["command"].as_str(), Some("pack"));
    assert_eq!(v["file_count"].as_u64(), Some(2));
    assert_eq!(v["app"].as_str(), Some("TestApp"));
    assert_eq!(v["portable"].as_null(), Some(()));
    let setup_name = v["setup"].as_str().expect("setup path in JSON");
    assert!(setup_name.ends_with("TestApp-v1.2.3-Setup.exe"));
    let files = v["files"].as_array().expect("files array");
    assert_eq!(files.len(), 2);
    // Without --portable the sums file covers only the Setup.exe.
    let sums_text = fs::read_to_string(out_dir.join("SHA256SUMS.txt")).expect("read sums");
    assert!(sums_text.contains("TestApp-v1.2.3-Setup.exe"));
    assert!(!sums_text.contains("Portable.zip"));
}

#[test]
fn validate_rejects_invalid_project_listing_every_error() {
    let dir = TempDir::new("invalid");
    let project = dir.join("bad.toml");
    fs::write(
        &project,
        "schema_version = 1\n\
         typewriter_interval_ms = 0\n\
         path_scope = \"bogus\"\n\
         \n\
         [app]\n\
         name = \"\"\n\
         version = \"1.0.0\"\n\
         publisher = \"\"\n\
         app_id = \"Bad Id!\"\n\
         main_exe = \"\"\n\
         description = \"\"\n\
         \n\
         [install]\n\
         scope = \"per-user\"\n\
         default_dir = \"{per-user-programs}\"\n\
         source_dir = \"\"\n\
         license_file = \"\"\n\
         require_admin = false\n\
         min_windows_build = 22000\n",
    )
    .expect("write bad project");
    let project_s = project.to_string_lossy().into_owned();

    let out = run(&["validate", &project_s]);
    assert_eq!(out.status.code(), Some(3), "invalid project should exit 3");
    let err = stderr(&out);
    assert!(
        err.contains("app.name"),
        "stderr lists the name error: {err}"
    );
    assert!(
        err.contains("app_id"),
        "stderr lists the app_id error: {err}"
    );
    assert!(
        err.contains("typewriter_interval_ms"),
        "stderr lists the typewriter error: {err}"
    );
    assert!(
        err.contains("path_scope"),
        "stderr lists the path_scope error: {err}"
    );

    // JSON mode: errors arrive as JSON with ok=false.
    let out = run(&["validate", &project_s, "--json"]);
    assert_eq!(out.status.code(), Some(3));
    let v = json_value(&out);
    assert_eq!(v["ok"].as_bool(), Some(false));
    assert_eq!(v["command"].as_str(), Some("validate"));
    let errors = v["errors"].as_array().expect("errors array");
    assert!(
        errors.len() >= 4,
        "all validation errors are listed: {errors:?}"
    );
}

#[test]
fn pack_rejects_invalid_project() {
    let dir = TempDir::new("packinvalid");
    let (_project, stub) = write_fixture(&dir);
    let bad = dir.join("bad.toml");
    fs::write(&bad, "[app]\nname = \"\"\n").expect("write bad project");
    let bad_s = bad.to_string_lossy().into_owned();
    let stub_s = stub.to_string_lossy().into_owned();
    let out = run(&["pack", &bad_s, "--stub", &stub_s]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "pack of invalid project should exit 3"
    );
    assert!(stderr(&out).contains("app.name"));
}

#[test]
fn usage_errors_exit_2_and_missing_files_exit_4() {
    let out = run(&["bogus-command"]);
    assert_eq!(out.status.code(), Some(2), "unknown command should exit 2");
    let out = run(&["pack", "some.toml"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "pack without --stub should exit 2"
    );
    let out = run(&["validate"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "validate without a file should exit 2"
    );
    let out = run(&["extract", "some.exe"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "extract without --out should exit 2"
    );
    let out = run(&["validate", "definitely-does-not-exist.toml"]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "missing project file should exit 4"
    );
}

//! Embed Windows resources: a manifest, version info, and an icon.
//!
//! Rust has no resource compiler, and adding one (or requiring `rc.exe` /
//! `windres` on PATH) would make the build fragile. So this script emits the
//! `.res` container itself — a small, stable format — and hands it to the
//! linker with `cargo:rustc-link-arg`, which both `link.exe` and `lld-link`
//! accept.
//!
//! The three resources and why they matter:
//!
//! * **Manifest** — declares `supportedOS` (Windows 10/11), `longPathAware`,
//!   `activeCodePage` and Per-Monitor-V2 DPI awareness, so Windows stops
//!   applying compatibility shims. It requests `asInvoker`: the stub
//!   elevates through UAC only when a project asks for a per-machine
//!   install, so demanding admin up front would break per-user installs.
//! * **Version info** — file/product version and names, which Explorer's
//!   Properties → Details and Apps & Features read.
//! * **Icon** — so the installer is not the generic blank icon in Explorer,
//!   on the taskbar, or in Apps & Features.
//!
//! All of it is a no-op off Windows, where the stub is only a placeholder.
//!
//! This file is shared: each crate's `build.rs` pulls it in with
//! `#[path = "../../resources/win_resources.rs"] mod win_resources;`, so the
//! two build scripts stay dependency-free and cannot drift apart.

use std::path::PathBuf;

/// Identifies which binary is being built: it changes the product name, the
/// original filename, the manifest's assembly identity and the icon glyph.
pub struct AppInfo {
    /// e.g. `RustPacker Setup`
    pub product: &'static str,
    pub description: &'static str,
    /// File name Explorer should report, e.g. `rustpacker-stub.exe`.
    pub exe_name: &'static str,
    /// Manifest `assemblyIdentity` name; must be unique per binary.
    pub assembly: &'static str,
    /// Icon glyph: `IconGlyph::Install` draws a `>_` prompt, `IconGlyph::Build`
    /// draws a window with a highlight bar.
    pub glyph: IconGlyph,
}

/// Only one variant is ever constructed per binary (the stub draws `Install`,
/// the GUI draws `Build`), but this file is compiled into both build scripts,
/// so the other variant looks dead to `-D dead-code` in each.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum IconGlyph {
    Install,
    Build,
}

/// Entry point for a `build.rs`.
///
/// Emits the resources and tells cargo to pass them to the linker. On a
/// non-Windows target — where these binaries are only placeholders — it
/// returns without doing anything.
pub fn emit(app: AppInfo) {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../resources/win_resources.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let res_path = out_dir.join("rustpacker.res");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());

    let icon = icon_bytes(app.glyph);
    let group = group_icon_bytes(&icon);
    let resources = [
        (RT_MANIFEST, 1u32, manifest_bytes(&app)),
        (RT_ICON, 1, icon),
        (RT_GROUP_ICON, 1, group),
        (RT_VERSION, 1, version_info_bytes(&version, &app)),
    ];

    std::fs::write(&res_path, build_res(&resources)).expect("write .res");
    println!("cargo:rustc-link-arg={}", res_path.display());
}

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const RT_VERSION: u32 = 16;
const RT_MANIFEST: u32 = 24;

// ---------------------------------------------------------------------------
// .res container
// ---------------------------------------------------------------------------
//
// A `.res` file is *not* the PE image's resource directory. It is a flat
// sequence of records, one per resource, which `cvtres`/`llvm-cvtres` later
// packs into the PE's three-level directory tree. Each record is (see
// `WinResHeaderPrefix` / `WinResHeaderSuffix` in LLVM's `WindowsResource.h`):
//
//   [u32 DataSize] [u32 HeaderSize]
//   [type] [name]                 each: u16 0xFFFF + u16 ordinal, or a
//                                 NUL-terminated UTF-16 string
//   [pad to 4]
//   [u32 DataVersion] [u16 MemoryFlags] [u16 language] [u32 version]
//   [u32 characteristics]
//   payload ...                   then padded to the next 4-byte boundary
//
// `HeaderSize` covers DataSize + HeaderSize + type + name + padding + suffix,
// i.e. the number of bytes from the start of the record to the payload, and
// `DataSize` is the unpadded payload length.
//
// Two requirements are easy to miss, and both make `link.exe`/`cvtres` reject
// the whole file with "invalid or corrupt file" (CVT1107 / LNK1136):
//
// 1. A 32-byte prologue: a 16-byte record-shaped header with `HeaderSize` 32
//    and zero size, then a 16-byte all-zero "null entry". Every reader skips
//    exactly those 32 bytes before the first real record.
// 2. `HeaderSize` must be at least 32 — LLVM's reader rejects anything smaller
//    (`MIN_HEADER_SIZE = 7 * u32 + 2 * u16`). With integer type and name ids
//    the header is exactly 32 bytes, so there is no room to be sloppy.
//
// Writing a PE-style directory here instead is what produced CVT1107 in the
// first place.

/// `MOVEABLE | PURE`: what `rc.exe` uses for manifests and version info.
const MEMORY_FLAGS: u16 = 0x0030;
/// 0x0409 (en-US), `LANG_ENGLISH | SUBLANG_ENGLISH_US`.
const RESOURCE_LANG: u16 = 0x0409;

fn build_res(resources: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(RES_PROLOGUE_LEN + 4096);

    // ---- prologue -------------------------------------------------------
    // A record-shaped header with no payload: DataSize 0, HeaderSize 32,
    // type 0, name 0. `cvtres` compares this against what it expects and
    // readers simply skip it.
    out.extend_from_slice(&0u32.to_le_bytes()); // DataSize
    out.extend_from_slice(&RES_PROLOGUE_LEN.to_le_bytes()); // HeaderSize
    write_res_identifier(&mut out, 0);
    write_res_identifier(&mut out, 0);
    out.extend_from_slice(&[0u8; RES_HEADER_SUFFIX_LEN as usize]); // suffix
    // ...followed by the 16-byte null entry.
    out.extend_from_slice(&[0u8; RES_NULL_ENTRY_LEN as usize]);

    for (res_type, res_id, data) in resources {
        let header_at = out.len();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
        out.extend_from_slice(&0u32.to_le_bytes()); // HeaderSize, patched below
        write_res_identifier(&mut out, *res_type);
        write_res_identifier(&mut out, *res_id);
        out.resize(out.len() + (4 - out.len() % 4) % 4, 0);

        let header_size = (out.len() - header_at) as u32 + RES_HEADER_SUFFIX_LEN;
        out[header_at + 4..header_at + 8].copy_from_slice(&header_size.to_le_bytes());

        out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
        out.extend_from_slice(&MEMORY_FLAGS.to_le_bytes());
        out.extend_from_slice(&RESOURCE_LANG.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // version
        out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
        out.extend_from_slice(data);
        out.resize(out.len() + (4 - data.len() % 4) % 4, 0);
    }
    out
}

/// Bytes after the type/name identifiers: DataVersion, MemoryFlags, language,
/// version and characteristics.
const RES_HEADER_SUFFIX_LEN: u32 = 4 + 2 + 2 + 4 + 4;
/// Bytes every `.res` starts with: the 16-byte magic header plus the null
/// entry. `MIN_HEADER_SIZE` in LLVM's reader is this same 32.
const RES_PROLOGUE_LEN: u32 = 32;
/// The all-zero record that follows the magic header.
const RES_NULL_ENTRY_LEN: u32 = 16;

/// A resource identifier: `0xFFFF` + ordinal, which is what `rc.exe` emits
/// for every resource this project uses (all of them are integer ids).
fn write_res_identifier(out: &mut Vec<u8>, id: u32) {
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&(id as u16).to_le_bytes());
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

fn manifest_bytes(app: &AppInfo) -> Vec<u8> {
    let manifest = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="{assembly}" version="1.0.0.0" processorArchitecture="*"/>
  <description>{description}</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
      <supportedOS Id="{{1f676c76-80e1-4239-95bb-83d0f6d0da78}}"/>
    </application>
  </compatibility>
</assembly>
"#,
        assembly = app.assembly,
        description = app.description,
    );
    manifest.into_bytes()
}

// ---------------------------------------------------------------------------
// Icon
// ---------------------------------------------------------------------------

const ICON_SIZE: u32 = 32;

/// A 32×32 32bpp icon as a bare DIB: a dark terminal window with a green
/// prompt, matching the wizard's One Dark palette.
fn icon_bytes(glyph: IconGlyph) -> Vec<u8> {
    // The AND mask is 1 bit per pixel, rows padded to 4 bytes. Written as
    // plain arithmetic because `div_ceil` needs Rust 1.73 and the workspace
    // supports 1.70.
    let mask_stride = ((ICON_SIZE + 31) / 32 * 4) as usize;
    let xor_len = (ICON_SIZE * ICON_SIZE * 4) as usize;
    let and_len = mask_stride * ICON_SIZE as usize;
    let dib_len = 40 + xor_len + and_len;

    let mut dib: Vec<u8> = Vec::with_capacity(dib_len);
    dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
    dib.extend_from_slice(&ICON_SIZE.to_le_bytes()); // biWidth
    dib.extend_from_slice(&(ICON_SIZE * 2).to_le_bytes()); // biHeight: XOR + AND
    dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    dib.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    dib.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    dib.extend_from_slice(&((xor_len + and_len) as u32).to_le_bytes()); // biSizeImage
    dib.extend_from_slice(&0i32.to_le_bytes()); // x pixels/metre
    dib.extend_from_slice(&0i32.to_le_bytes()); // y pixels/metre
    dib.extend_from_slice(&0u32.to_le_bytes()); // colours used
    dib.extend_from_slice(&0u32.to_le_bytes()); // important colours

    // Colour data is bottom-up; the AND mask does nothing because the alpha
    // channel already carries the shape.
    for y in (0..ICON_SIZE).rev() {
        for x in 0..ICON_SIZE {
            let (r, g, b, a) = icon_pixel(x, y, glyph);
            dib.extend_from_slice(&[b, g, r, a]);
        }
    }
    dib.resize(dib.len() + and_len, 0);

    // `RT_ICON` wants the bare DIB, **not** an `.ico` container: the loader
    // passes the resource bytes straight to `CreateIconFromResourceEx`. The
    // container with its 22-byte header belongs in an `.ico` file on disk,
    // and `RT_GROUP_ICON` (below) stands in for that header here.
    dib
}

/// Colour of the icon at `(x, y)` (origin top-left) as `(r, g, b, a)`.
fn icon_pixel(x: u32, y: u32, glyph: IconGlyph) -> (u8, u8, u8, u8) {
    const BG: (u8, u8, u8) = (0x28, 0x2C, 0x34);
    const SURFACE: (u8, u8, u8) = (0x21, 0x25, 0x2B);
    const ACCENT: (u8, u8, u8) = (0x98, 0xC3, 0x79);

    if !inside_rounded(x, y, 1, 1, 30, 30, 5) {
        return (0, 0, 0, 0);
    }
    if y < 7 {
        return (SURFACE.0, SURFACE.1, SURFACE.2, 255);
    }
    match glyph {
        IconGlyph::Install => {
            // `>_`: a wedge for the caret, a bar for the underscore.
            let caret = (7..=14).contains(&x)
                && (10..=20).contains(&y)
                && (x as i32 - 7) * 2 >= (y as i32 - 15).abs() - 2;
            let underscore = (13..=22).contains(&x) && (18..=20).contains(&y);
            if caret || underscore {
                return (ACCENT.0, ACCENT.1, ACCENT.2, 255);
            }
        }
        IconGlyph::Build => {
            // A builder: a frame plus a highlighted title bar stripe.
            let frame = (7..=24).contains(&x)
                && (11..=23).contains(&y)
                && ((x == 7 || x == 24) || (y == 11 || y == 23));
            let stripe = (11..=20).contains(&x) && (14..=15).contains(&y);
            if frame || stripe {
                return (ACCENT.0, ACCENT.1, ACCENT.2, 255);
            }
        }
    }
    (BG.0, BG.1, BG.2, 255)
}

/// True when `(x, y)` lies inside a rectangle with rounded corners.
fn inside_rounded(x: u32, y: u32, rx: u32, ry: u32, w: u32, h: u32, radius: u32) -> bool {
    if x < rx || y < ry || x >= rx + w || y >= ry + h {
        return false;
    }
    let dx = if x < rx + radius {
        rx + radius - x
    } else if x >= rx + w - radius {
        x - (rx + w - radius - 1)
    } else {
        0
    };
    let dy = if y < ry + radius {
        ry + radius - y
    } else if y >= ry + h - radius {
        y - (ry + h - radius - 1)
    } else {
        0
    };
    dx * dx + dy * dy <= radius * radius
}

/// `GRPICONDIR`: points Explorer at the single ICON resource.
/// The `RT_GROUP_ICON` payload: a `GRPICONDIR` with one `GRPICONDIRENTRY`,
/// which is what makes Explorer pick our icon for the file and the shell cache
/// it by resource id. `bytes_in_res` is the DIB length, matching `RT_ICON`.
fn group_icon_bytes(dib: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + 14);
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&1u16.to_le_bytes()); // count
    out.push(ICON_SIZE as u8);
    out.push(ICON_SIZE as u8);
    out.push(0); // palette colours
    out.push(0); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bit count
    out.extend_from_slice(&(dib.len() as u32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // resource id
    out
}

// ---------------------------------------------------------------------------
// Version info
// ---------------------------------------------------------------------------

/// `VS_VERSIONINFO`.
///
/// Every node is `wLength, wValueLength, wType, szKey\0, [pad], Value,
/// Children`, and each is padded to a 32-bit boundary. Building the children
/// first and wrapping them in the parent is the only way the recorded lengths
/// can be right, which is what makes this readable as a tree.
fn version_info_bytes(version: &str, app: &AppInfo) -> Vec<u8> {
    let (major, minor, patch) = parse_version(version);
    let build = 0u16;
    let version_string = format!("{major}.{minor}.{patch}.{build}");

    // VS_FIXEDFILEINFO, the binary version Explorer sorts by.
    let mut fixed = Vec::with_capacity(52);
    fixed.extend_from_slice(&0xFEEF_04BDu32.to_le_bytes()); // signature
    fixed.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // structure version
    fixed.extend_from_slice(&pack(major, minor).to_le_bytes()); // file version MS
    fixed.extend_from_slice(&pack(patch, build).to_le_bytes()); // file version LS
    fixed.extend_from_slice(&pack(major, minor).to_le_bytes()); // product version MS
    fixed.extend_from_slice(&pack(patch, build).to_le_bytes()); // product version LS
    fixed.extend_from_slice(&0x0000_003Fu32.to_le_bytes()); // file flags mask
    fixed.extend_from_slice(&0u32.to_le_bytes()); // file flags
    fixed.extend_from_slice(&0x0004_0004u32.to_le_bytes()); // VOS_NT_WINDOWS32
    fixed.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // VFT_APP
    fixed.extend_from_slice(&0u32.to_le_bytes()); // file subtype
    fixed.extend_from_slice(&0u32.to_le_bytes()); // file date

    let mut strings = Vec::new();
    for (key, value) in [
        ("CompanyName", "Sean Shaffer"),
        ("FileDescription", app.description),
        ("FileVersion", version_string.as_str()),
        ("InternalName", app.exe_name),
        ("LegalCopyright", "MIT License"),
        ("OriginalFilename", app.exe_name),
        ("ProductName", app.product),
        ("ProductVersion", version_string.as_str()),
    ] {
        strings.extend(vs_block(1, key, Some(&utf16_value(value))));
    }
    // 0x0409 = en-US, 0x04B0 = 1200 = Unicode.
    // 0x040904B0 is a container node, not a text value: its children are the
    // individual strings, so wValueLength must be 0.
    let table = vs_node(1, "040904B0", None, &strings);
    let string_file_info = vs_node(1, "StringFileInfo", None, &table);

    let mut translation = Vec::with_capacity(4);
    translation.extend_from_slice(&0x0409u16.to_le_bytes());
    translation.extend_from_slice(&1200u16.to_le_bytes());
    let var_file_info = vs_node(
        1,
        "VarFileInfo",
        None,
        &vs_block(0, "Translation", Some(&translation)),
    );

    // The root carries VS_FIXEDFILEINFO as its value and the two info blocks
    // as children. Only `wLength` covers the children.
    let mut child_nodes = Vec::new();
    child_nodes.extend_from_slice(&string_file_info);
    child_nodes.extend_from_slice(&var_file_info);
    vs_node(0, "VS_VERSION_INFO", Some(&fixed), &child_nodes)
}

fn pack(hi: u16, lo: u16) -> u32 {
    ((hi as u32) << 16) | lo as u32
}

fn parse_version(version: &str) -> (u16, u16, u16) {
    let mut parts = version.split('.').map(|p| p.parse::<u16>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// One UTF-16LE string plus its NUL terminator.
fn utf16_value(value: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() * 2 + 2);
    for unit in value.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// One length-prefixed UTF-16 node with an optional value and children,
/// padded to a 32-bit boundary.
///
/// `wValueLength` counts **UTF-16 code units** for text nodes (`w_type == 1`)
/// and **bytes** for binary nodes — that distinction is what the loader
/// checks. Children are not counted in `wValueLength` (only in `wLength`),
/// which is why the `VS_VERSION_INFO` root passes `None` here and appends its
/// children afterwards.
fn vs_block(w_type: u16, key: &str, value: Option<&[u8]>) -> Vec<u8> {
    vs_node(w_type, key, value, &[])
}

/// [`vs_block`] plus child nodes.
fn vs_node(w_type: u16, key: &str, value: Option<&[u8]>, children: &[u8]) -> Vec<u8> {
    let value_bytes = value.unwrap_or(&[]);
    // Text values are NUL-terminated UTF-16; ordinal 1 means "text".
    let value_len = if w_type == 1 {
        value_bytes.len() / 2
    } else {
        value_bytes.len()
    };

    let key_bytes = utf16_value(key);
    let mut out = Vec::with_capacity(6 + key_bytes.len() + value_bytes.len() + children.len() + 4);
    out.extend_from_slice(&0u16.to_le_bytes()); // wLength, patched below
    out.extend_from_slice(&(value_len as u16).to_le_bytes()); // wValueLength
    out.extend_from_slice(&w_type.to_le_bytes()); // wType
    out.extend_from_slice(&key_bytes);
    out.extend_from_slice(value_bytes);
    // The value is padded to 4 bytes; children start on that boundary.
    out.resize(out.len() + (4 - (out.len() % 4)) % 4, 0);
    out.extend_from_slice(children);

    let total = out.len();
    out[0..2].copy_from_slice(&(total as u16).to_le_bytes());
    out
}

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

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let res_path = out_dir.join("rustpacker-stub.res");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());

    let resources = [
        (RT_MANIFEST, 1u32, manifest_bytes()),
        (RT_ICON, 1, icon_bytes()),
        (RT_GROUP_ICON, 1, group_icon_bytes()),
        (RT_VERSION, 1, version_info_bytes(&version)),
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
// Layout:
//   IMAGE_RESOURCE_DIRECTORY            (root, 16 bytes + 8 per type)
//     IMAGE_RESOURCE_DIRECTORY          (one per resource type)
//       IMAGE_RESOURCE_DIRECTORY        (one per resource name/id)
//         IMAGE_RESOURCE_DATA_ENTRY     -> offset/size in the data section
//   ...data section: [size][payload][padding] per resource...
//
// The root has no names (everything is an ordinal id), and every level is
// 32-bit aligned. Offsets in directory entries are relative to the start of
// the file; the high bit marks a subdirectory instead of a leaf.

const DIR_ENTRY_LEN: usize = 8;
const DIR_HEADER_LEN: usize = 16;
const DATA_ENTRY_LEN: usize = 16;
/// Directory-entry flag: the offset points at another directory.
const SUBDIR_FLAG: u32 = 0x8000_0000;
/// Name field flag: the value is an ordinal id, not an offset into the names.
const ID_FLAG: u32 = 0x8000_0000;

fn build_res(resources: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    // Resource types, in first-seen order, each with its (id, data) children.
    let mut types: Vec<(u32, Vec<(u32, Vec<u8>)>)> = Vec::new();
    for (res_type, res_id, data) in resources {
        match types.iter_mut().find(|(t, _)| t == res_type) {
            Some((_, children)) => children.push((*res_id, data.clone())),
            None => types.push((*res_type, vec![(*res_id, data.clone())])),
        }
    }

    // Every directory at a level is the same size, so the layout can be
    // computed in one pass before anything is written.
    let root_len = DIR_HEADER_LEN + DIR_ENTRY_LEN * types.len();
    let type_dirs_len: usize = types
        .iter()
        .map(|(_, children)| DIR_HEADER_LEN + DIR_ENTRY_LEN * children.len())
        .sum();
    let leaf_dirs_len: usize = types
        .iter()
        .map(|(_, children)| DIR_HEADER_LEN + DIR_ENTRY_LEN * children.len())
        .sum();
    let data_entries_len = DATA_ENTRY_LEN * resources.len();

    let root_off = 0usize;
    let type_dirs_off = root_off + root_len;
    let leaf_dirs_off = type_dirs_off + type_dirs_len;
    let data_entries_off = leaf_dirs_off + leaf_dirs_len;
    let data_off = data_entries_off + data_entries_len;

    // Data section, recording each resource's offset and size as it is laid out.
    let mut data: Vec<u8> = Vec::new();
    let mut payload_offsets: Vec<((u32, u32), u32, u32)> = Vec::new();
    for (res_type, res_id, bytes) in resources {
        let offset = (data_off + data.len()) as u32;
        payload_offsets.push(((*res_type, *res_id), offset, bytes.len() as u32));
        data.extend_from_slice(bytes);
        let pad = (4 - (bytes.len() % 4)) % 4;
        data.extend(std::iter::repeat(0u8).take(pad));
    }

    let mut out: Vec<u8> = Vec::with_capacity(data_off + data.len());

    // ---- root directory -------------------------------------------------
    write_dir_header(&mut out, 0, types.len());
    let mut type_dir_cursor = type_dirs_off;
    for (res_type, children) in &types {
        write_dir_entry(&mut out, *res_type, (type_dir_cursor as u32) | SUBDIR_FLAG);
        type_dir_cursor += DIR_HEADER_LEN + DIR_ENTRY_LEN * children.len();
    }

    // ---- one directory per type ----------------------------------------
    let mut leaf_cursor = leaf_dirs_off;
    for (_, children) in &types {
        write_dir_header(&mut out, 0, children.len());
        for (res_id, _) in children {
            write_dir_entry(&mut out, *res_id, (leaf_cursor as u32) | SUBDIR_FLAG);
            leaf_cursor += DIR_HEADER_LEN + DIR_ENTRY_LEN;
        }
    }

    // ---- one directory per resource name -------------------------------
    // Each has a single unnamed child whose offset points at the data entry.
    let mut data_entry_cursor = data_entries_off;
    for (_, children) in &types {
        for _ in children {
            write_dir_header(&mut out, 1, 0);
            write_dir_entry(&mut out, 0, 0); // patched below with the real offset
            let patch_at = out.len() - 4;
            let real = (data_entry_cursor as u32).to_le_bytes();
            out[patch_at..patch_at + 4].copy_from_slice(&real);
            data_entry_cursor += DATA_ENTRY_LEN;
        }
    }

    // ---- data entries ---------------------------------------------------
    for (_, offset, size) in &payload_offsets {
        out.extend_from_slice(offset.to_le_bytes());
        out.extend_from_slice(size.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // code page
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    }

    debug_assert_eq!(out.len(), data_off, "directory layout drifted");
    out.extend_from_slice(&data);
    out
}

fn write_dir_header(out: &mut Vec<u8>, named: usize, ids: usize) {
    out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    out.extend_from_slice(&0u32.to_le_bytes()); // timestamp
    out.extend_from_slice(&0u32.to_le_bytes()); // major/minor version
    out.extend_from_slice(&(named as u16).to_le_bytes()); // named entries
    out.extend_from_slice(&(ids as u16).to_le_bytes()); // id entries
}

fn write_dir_entry(out: &mut Vec<u8>, id: u32, offset: u32) {
    out.extend_from_slice(&(ID_FLAG | id).to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

fn manifest_bytes() -> Vec<u8> {
    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="RustPacker.Setup" version="1.0.0.0" processorArchitecture="*"/>
  <description>RustPacker Setup</description>
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
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
    </application>
  </compatibility>
</assembly>
"#;
    MANIFEST.as_bytes().to_vec()
}

// ---------------------------------------------------------------------------
// Icon
// ---------------------------------------------------------------------------

const ICON_SIZE: u32 = 32;

/// A 32×32 32bpp icon in ICO form: a dark terminal window with a green
/// prompt, matching the wizard's One Dark palette.
fn icon_bytes() -> Vec<u8> {
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
            let (r, g, b, a) = icon_pixel(x, y);
            dib.extend_from_slice(&[b, g, r, a]);
        }
    }
    dib.extend(std::iter::repeat(0u8).take(and_len));

    let mut ico = Vec::with_capacity(6 + 16 + dib.len());
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    ico.extend_from_slice(&1u16.to_le_bytes()); // count
    ico.push(ICON_SIZE as u8);
    ico.push(ICON_SIZE as u8);
    ico.push(0); // palette colours
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bit count
    ico.extend_from_slice(&(dib.len() as u32).to_le_bytes());
    ico.extend_from_slice(&(6 + 16u32).to_le_bytes()); // DIB offset
    ico.extend_from_slice(&dib);
    ico
}

/// Colour of the icon at `(x, y)` (origin top-left) as `(r, g, b, a)`.
fn icon_pixel(x: u32, y: u32) -> (u8, u8, u8, u8) {
    const BG: (u8, u8, u8) = (0x28, 0x2C, 0x34);
    const SURFACE: (u8, u8, u8) = (0x21, 0x25, 0x2B);
    const ACCENT: (u8, u8, u8) = (0x98, 0xC3, 0x79);

    if !inside_rounded(x, y, 1, 1, 30, 30, 5) {
        return (0, 0, 0, 0);
    }
    if y < 7 {
        return (SURFACE.0, SURFACE.1, SURFACE.2, 255);
    }
    // A `>_` prompt: a wedge for the caret, a bar for the underscore.
    let caret = (7..=14).contains(&x)
        && y >= 10
        && y <= 20
        && (x as i32 - 7) * 2 >= (y as i32 - 15).abs() - 2
        && (14 - x as i32) > 0;
    let underscore = (13..=22).contains(&x) && (18..=20).contains(&y);
    if caret || underscore {
        return (ACCENT.0, ACCENT.1, ACCENT.2, 255);
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
fn group_icon_bytes() -> Vec<u8> {
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
    out.extend_from_slice(&(icon_bytes().len() as u32).to_le_bytes());
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
fn version_info_bytes(version: &str) -> Vec<u8> {
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
        ("FileDescription", "RustPacker Setup"),
        ("FileVersion", version_string.as_str()),
        ("InternalName", "rustpacker-stub"),
        ("LegalCopyright", "MIT License"),
        ("OriginalFilename", "rustpacker-stub.exe"),
        ("ProductName", "RustPacker"),
        ("ProductVersion", version_string.as_str()),
    ] {
        strings.extend(vs_block(1, key, Some(&utf16_value(value))));
    }
    // 0x0409 = en-US, 0x04B0 = 1200 = Unicode.
    let string_file_info = vs_block(
        1,
        "StringFileInfo",
        Some(&vs_block(1, "040904B0", Some(&strings))),
    );

    let mut translation = Vec::with_capacity(4);
    translation.extend_from_slice(&0x0409u16.to_le_bytes());
    translation.extend_from_slice(&1200u16.to_le_bytes());
    let var_file_info = vs_block(
        1,
        "VarFileInfo",
        Some(&vs_block(0, "Translation", Some(&translation))),
    );

    let mut children = Vec::new();
    children.extend(vs_block(0, "VS_VERSION_INFO", Some(&fixed)));
    children.extend(string_file_info);
    children.extend(var_file_info);
    // The outermost node's children are its blocks; `vs_block` pads the value
    // it is given, so passing the concatenation keeps the alignment right.
    vs_block(1, "VS_VERSION_INFO", Some(&children))
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

/// One length-prefixed UTF-16 node plus its value, padded to 4 bytes.
///
/// `w_value_len` counts UTF-16 code units for text nodes (`w_type == 1`) and
/// bytes for binary nodes, which is the distinction the loader checks.
fn vs_block(w_type: u16, key: &str, value: Option<&[u8]>) -> Vec<u8> {
    let value_bytes = value.unwrap_or(&[]);
    let value_pad = (4 - (value_bytes.len() % 4)) % 4;
    let value_len = if w_type == 1 {
        value_bytes.len() / 2
    } else {
        value_bytes.len()
    };

    let key_bytes = utf16_value(key);
    let total = 6 + key_bytes.len() + value_bytes.len() + value_pad;

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u16).to_le_bytes()); // wLength
    out.extend_from_slice(&(value_len as u16).to_le_bytes()); // wValueLength
    out.extend_from_slice(&w_type.to_le_bytes()); // wType
    out.extend_from_slice(&key_bytes);
    out.extend_from_slice(value_bytes);
    out.extend(std::iter::repeat(0u8).take(value_pad));
    out
}

//! Payload packer / extractor.
//!
//! A generated `Setup.exe` is the stub PE with one appended payload:
//!
//! ```text
//! [ stub PE bytes ][ zstd archive ][ footer ]
//! footer (72 bytes):
//!   0..16   magic  "RUSTPACKER_OVL01"
//!   16..24  payload offset (u64 LE, from start of file)
//!   24..32  payload length (u64 LE)
//!   32..64  SHA-256 of the payload bytes
//!   64..72  archive (uncompressed) length (u64 LE)
//! ```
//!
//! The uncompressed archive is:
//! ```text
//! u32 file_count
//! per file: u16 path_len, path UTF-8 (forward slashes, relative),
//!           u64 data_len, 32-byte SHA-256 of data, data bytes
//! ```
//! The project config travels inside the archive as the reserved entry
//! [`.rustpacker/config.json`](CONFIG_ENTRY_NAME); extractors surface it
//! as the manifest instead of writing it to disk as a payload file.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const OVERLAY_MAGIC: &[u8; 16] = b"RUSTPACKER_OVL01";
pub const FOOTER_LEN: usize = 72;
pub const CONFIG_ENTRY_NAME: &str = ".rustpacker/config.json";
const DEFAULT_ZSTD_LEVEL: i32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative path inside the payload, forward slashes.
    pub path: String,
    pub size: u64,
    /// Lowercase hex SHA-256 of the file data.
    pub sha256_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadManifest {
    pub files: Vec<FileEntry>,
    /// The embedded project config JSON, when the packer stored one.
    pub config_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedInfo {
    pub payload_offset: u64,
    pub payload_len: u64,
    pub archive_len: u64,
    pub payload_sha256_hex: String,
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Reject absolute paths and any `..` / root components so a crafted
/// payload can never write outside the install directory.
pub fn sanitise_rel_path(path: &str) -> Result<PathBuf, String> {
    let normalised = path.replace('\\', "/");
    if normalised.is_empty() {
        return Err("payload entry has an empty path".into());
    }
    let p = Path::new(&normalised);
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return Err(format!("payload entry path escapes its root: {path}")),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!("payload entry has no usable path: {path}"));
    }
    Ok(out)
}

struct ArchiveFile {
    path: String,
    data: Vec<u8>,
}

fn build_archive(files: &[ArchiveFile]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for f in files {
        let path_bytes = f.path.as_bytes();
        out.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(path_bytes);
        out.extend_from_slice(&(f.data.len() as u64).to_le_bytes());
        out.extend_from_slice(&Sha256::digest(&f.data));
        out.extend_from_slice(&f.data);
    }
    out
}

fn parse_archive(archive: &[u8]) -> Result<Vec<ArchiveFile>, String> {
    let mut cur = Cursor {
        buf: archive,
        pos: 0,
    };
    let count = cur.u32()? as usize;
    let mut files = Vec::with_capacity(count.min(1_000_000));
    for _ in 0..count {
        let path_len = cur.u16()? as usize;
        let path_bytes = cur.take(path_len)?.to_vec();
        let path = String::from_utf8(path_bytes)
            .map_err(|_| "payload entry path is not UTF-8".to_string())?;
        sanitise_rel_path(&path)?;
        let data_len = cur.u64()? as usize;
        let expected = cur.take(32)?.to_vec();
        let data = cur.take(data_len)?.to_vec();
        if Sha256::digest(&data).as_slice() != expected.as_slice() {
            return Err(format!("SHA-256 mismatch for payload entry {path}"));
        }
        files.push(ArchiveFile { path, data });
    }
    Ok(files)
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or("payload archive is truncated")?;
        if end > self.buf.len() {
            return Err("payload archive is truncated".into());
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
}

/// Append a payload built from `files` ((relative path, data) pairs) plus
/// the project config JSON to `stub` bytes, returning the finished exe.
pub fn pack_files(
    stub: &[u8],
    mut files: Vec<(String, Vec<u8>)>,
    config_json: &str,
) -> Result<Vec<u8>, String> {
    for (path, _) in &files {
        sanitise_rel_path(path)?;
        if path.replace('\\', "/") == CONFIG_ENTRY_NAME {
            return Err(format!(
                "payload source must not contain the reserved entry {CONFIG_ENTRY_NAME}"
            ));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut archive_files: Vec<ArchiveFile> = files
        .into_iter()
        .map(|(path, data)| ArchiveFile {
            path: path.replace('\\', "/"),
            data,
        })
        .collect();
    archive_files.push(ArchiveFile {
        path: CONFIG_ENTRY_NAME.into(),
        data: config_json.as_bytes().to_vec(),
    });

    let archive = build_archive(&archive_files);
    let archive_len = archive.len() as u64;
    let payload = zstd::encode_all(&archive[..], DEFAULT_ZSTD_LEVEL)
        .map_err(|e| format!("payload compression failed: {e}"))?;

    let payload_offset = stub.len() as u64;
    let payload_len = payload.len() as u64;
    let mut out = Vec::with_capacity(stub.len() + payload.len() + FOOTER_LEN);
    out.extend_from_slice(stub);
    out.extend_from_slice(&payload);
    out.extend_from_slice(OVERLAY_MAGIC);
    out.extend_from_slice(&payload_offset.to_le_bytes());
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(&Sha256::digest(&payload));
    out.extend_from_slice(&archive_len.to_le_bytes());
    Ok(out)
}

/// Walk a source directory (sorted, deterministic) and pack it onto `stub`.
pub fn pack_directory(
    stub: &[u8],
    source_dir: &Path,
    config_json: &str,
) -> Result<(Vec<u8>, Vec<FileEntry>), String> {
    if !source_dir.is_dir() {
        return Err(format!(
            "payload source directory not found: {}",
            source_dir.display()
        ));
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_files(source_dir, source_dir, &mut paths)?;
    paths.sort();
    let mut files = Vec::new();
    let mut entries = Vec::new();
    for path in paths {
        let rel = path
            .strip_prefix(source_dir)
            .map_err(|_| "internal path error".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let data =
            std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        entries.push(FileEntry {
            path: rel.clone(),
            size: data.len() as u64,
            sha256_hex: sha256_hex(&data),
        });
        files.push((rel, data));
    }
    let exe = pack_files(stub, files, config_json)?;
    Ok((exe, entries))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut read = std::fs::read_dir(dir)
        .map_err(|e| format!("could not list {}: {e}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("could not list {}: {e}", dir.display()))?;
    read.sort_by_key(|e| e.file_name());
    for entry in read {
        let path = entry.path();
        let _ = root;
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Locate and verify the payload footer of a packed exe.
pub fn find_payload(exe: &[u8]) -> Result<PackedInfo, String> {
    if exe.len() < FOOTER_LEN {
        return Err("file is too small to carry a RustPacker payload".into());
    }
    let footer = &exe[exe.len() - FOOTER_LEN..];
    if &footer[0..16] != OVERLAY_MAGIC {
        return Err("no RustPacker payload found (overlay marker missing)".into());
    }
    let payload_offset = u64::from_le_bytes(footer[16..24].try_into().map_err(|_| "bad footer")?);
    let payload_len = u64::from_le_bytes(footer[24..32].try_into().map_err(|_| "bad footer")?);
    let archive_len = u64::from_le_bytes(footer[64..72].try_into().map_err(|_| "bad footer")?);
    let end = payload_offset
        .checked_add(payload_len)
        .filter(|e| *e as usize == exe.len() - FOOTER_LEN)
        .ok_or("payload bounds in footer do not match the file size")?;
    let _ = end;
    let payload = &exe[payload_offset as usize..(payload_offset + payload_len) as usize];
    if Sha256::digest(payload).as_slice() != &footer[32..64] {
        return Err("payload SHA-256 mismatch — the file is corrupt or was modified".into());
    }
    Ok(PackedInfo {
        payload_offset,
        payload_len,
        archive_len,
        payload_sha256_hex: sha256_hex(payload),
    })
}

fn decode_payload(exe: &[u8]) -> Result<Vec<ArchiveFile>, String> {
    let info = find_payload(exe)?;
    let payload =
        &exe[info.payload_offset as usize..(info.payload_offset + info.payload_len) as usize];
    let mut decoder =
        zstd::Decoder::new(payload).map_err(|e| format!("payload decompression failed: {e}"))?;
    let mut archive = Vec::with_capacity(info.archive_len as usize);
    decoder
        .read_to_end(&mut archive)
        .map_err(|e| format!("payload decompression failed: {e}"))?;
    if archive.len() as u64 != info.archive_len {
        return Err("payload archive length mismatch".into());
    }
    parse_archive(&archive)
}

/// Read the manifest (file list + embedded config) without extracting.
pub fn read_manifest(exe: &[u8]) -> Result<PayloadManifest, String> {
    let files = decode_payload(exe)?;
    let mut entries = Vec::new();
    let mut config_json = None;
    for f in files {
        if f.path == CONFIG_ENTRY_NAME {
            config_json = Some(
                String::from_utf8(f.data)
                    .map_err(|_| "embedded config is not UTF-8".to_string())?,
            );
        } else {
            entries.push(FileEntry {
                path: f.path,
                size: f.data.len() as u64,
                sha256_hex: sha256_hex(&f.data),
            });
        }
    }
    Ok(PayloadManifest {
        files: entries,
        config_json,
    })
}

/// Verify the whole payload (footer hash, decompression, per-file hashes).
pub fn verify_payload(exe: &[u8]) -> Result<PayloadManifest, String> {
    read_manifest(exe)
}

/// Extract every payload file under `dest_dir`. The embedded config entry
/// is not written to disk. Returns the extracted relative paths.
pub fn extract_payload(exe: &[u8], dest_dir: &Path) -> Result<Vec<String>, String> {
    let files = decode_payload(exe)?;
    let mut extracted = Vec::new();
    for f in files {
        if f.path == CONFIG_ENTRY_NAME {
            continue;
        }
        let rel = sanitise_rel_path(&f.path)?;
        let target = dest_dir.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        let mut file = std::fs::File::create(&target)
            .map_err(|e| format!("could not create {}: {e}", target.display()))?;
        file.write_all(&f.data)
            .map_err(|e| format!("could not write {}: {e}", target.display()))?;
        extracted.push(f.path);
    }
    Ok(extracted)
}

/// Write a `SHA256SUMS.txt` body for (name, bytes) pairs.
pub fn sha256sums_txt(items: &[(&str, &[u8])]) -> String {
    let mut out = String::new();
    for (name, data) in items {
        out.push_str(&format!("{}  {}\n", sha256_hex(data), name));
    }
    out
}

/// Write stored (uncompressed) ZIP entries — the portable export. ZIP is
/// written by hand so the packer needs no extra dependency; entries use
/// the standard stored method with a correct CRC-32.
pub fn write_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in entries {
        let name_bytes = name.replace('\\', "/");
        let name_bytes = name_bytes.as_bytes();
        let crc = crc32(data);
        let offset = out.len() as u32;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // date 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x21u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    let central_offset = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub() -> Vec<u8> {
        let mut v = b"MZ".to_vec();
        v.extend(std::iter::repeat(0u8).take(512));
        v
    }

    #[test]
    fn pack_find_verify_extract_round_trip() {
        let exe = pack_files(
            &stub(),
            vec![
                ("app.exe".into(), b"binary-bytes".to_vec()),
                ("docs/readme.txt".into(), b"hello rustpacker".to_vec()),
            ],
            "{\"app\":{}}",
        )
        .expect("pack");
        let info = find_payload(&exe).expect("find");
        assert_eq!(info.payload_offset as usize, stub().len());
        let manifest = read_manifest(&exe).expect("manifest");
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.config_json.as_deref(), Some("{\"app\":{}}"));
        assert_eq!(manifest.files[0].path, "app.exe");
        assert_eq!(manifest.files[0].sha256_hex, sha256_hex(b"binary-bytes"));

        let dir =
            std::env::temp_dir().join(format!("rustpacker-payload-test-{}", std::process::id()));
        let extracted = extract_payload(&exe, &dir).expect("extract");
        assert_eq!(extracted.len(), 2);
        assert_eq!(
            std::fs::read(dir.join("app.exe")).expect("read"),
            b"binary-bytes"
        );
        assert_eq!(
            std::fs::read(dir.join("docs/readme.txt")).expect("read"),
            b"hello rustpacker"
        );
        assert!(
            !dir.join(CONFIG_ENTRY_NAME).exists(),
            "config entry is not written to disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let mut exe =
            pack_files(&stub(), vec![("a.txt".into(), b"data".to_vec())], "{}").expect("pack");
        let pos = exe.len() - FOOTER_LEN - 1;
        exe[pos] ^= 0xFF;
        assert!(find_payload(&exe).is_err());
    }

    #[test]
    fn unpacked_file_is_rejected() {
        assert!(find_payload(&stub()).is_err());
        assert!(find_payload(b"tiny").is_err());
    }

    #[test]
    fn traversal_paths_are_rejected() {
        assert!(sanitise_rel_path("../evil.exe").is_err());
        assert!(sanitise_rel_path("a/../../evil.exe").is_err());
        assert!(sanitise_rel_path("/abs/path").is_err());
        assert!(pack_files(&stub(), vec![("../evil".into(), vec![])], "{}").is_err());
        assert!(sanitise_rel_path("sub/ok.txt").is_ok());
    }

    #[test]
    fn pack_directory_round_trip() {
        let base = std::env::temp_dir().join(format!("rustpacker-packdir-{}", std::process::id()));
        let src = base.join("src");
        std::fs::create_dir_all(src.join("sub")).expect("mkdir");
        std::fs::write(src.join("main.exe"), b"exe").expect("write");
        std::fs::write(src.join("sub/data.bin"), b"bin").expect("write");
        let (exe, entries) = pack_directory(&stub(), &src, "{}").expect("pack dir");
        assert_eq!(entries.len(), 2);
        let out = base.join("out");
        extract_payload(&exe, &out).expect("extract");
        assert_eq!(
            std::fs::read(out.join("sub/data.bin")).expect("read"),
            b"bin"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn stored_zip_has_valid_structure() {
        let zip = write_stored_zip(&[("a.txt", b"abc"), ("b/c.txt", b"defg")]);
        assert_eq!(&zip[0..4], b"PK\x03\x04");
        // End-of-central-directory signature present near the tail.
        assert!(zip.windows(4).any(|w| w == b"PK\x05\x06"));
        assert_eq!(crc32(b"abc"), 0x3524_41C2);
    }
}

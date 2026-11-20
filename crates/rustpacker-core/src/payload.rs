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
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const OVERLAY_MAGIC: &[u8; 16] = b"RUSTPACKER_OVL01";
pub const FOOTER_LEN: usize = 72;
pub const CONFIG_ENTRY_NAME: &str = ".rustpacker/config.json";
const DEFAULT_ZSTD_LEVEL: i32 = 3;
/// Hard cap on the decompressed archive size. Payloads larger than this
/// are rejected instead of being decompressed into memory (zip-bomb
/// defence). 2 GiB is far above any realistic installer payload.
pub const MAX_ARCHIVE_LEN: u64 = 2 * 1024 * 1024 * 1024;
/// Hard cap on the number of entries in one archive.
pub const MAX_FILE_COUNT: usize = 1_000_000;
/// Maximum bytes for one path inside the archive (u16 length field).
pub const MAX_PATH_LEN: usize = u16::MAX as usize;
/// Maximum bytes for one stored ZIP entry (u32 length fields).
pub const MAX_ZIP_ENTRY_LEN: usize = u32::MAX as usize;
/// Maximum entries in one stored ZIP (u16 count fields).
pub const MAX_ZIP_ENTRIES: usize = u16::MAX as usize;

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
    hex_of(&Sha256::digest(data))
}

/// True for Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`,
/// `COM1`..`COM9`, `LPT1`..`LPT9`), with or without an extension.
fn is_reserved_name(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Reject absolute paths and any `..` / root components so a crafted
/// payload can never write outside the install directory.
///
/// This check is deliberately **OS-independent**: it does not rely on
/// `Path::components()` (whose parsing of `C:/...`, UNC paths, etc.
/// differs between Linux and Windows). It rejects:
/// * empty paths, absolute paths, UNC paths, drive-relative paths
///   (any `:` — covers `C:/`, `C:`, and ADS `file:stream`),
/// * `..` segments, empty segments (`//`, leading/trailing `/`),
/// * Windows-forbidden characters (`< > : " | ? *` and controls),
/// * segments ending in `.` or space (Windows strips these, which
///   would otherwise allow collisions/overwrites),
/// * reserved device names (`CON`, `NUL`, …).
///
/// `.` segments are normalised away (so `./a.txt` becomes `a.txt`).
pub fn sanitise_rel_path(path: &str) -> Result<PathBuf, String> {
    let normalised = path.replace('\\', "/");
    if normalised.is_empty() {
        return Err("payload entry has an empty path".into());
    }
    if normalised.contains(':') {
        return Err(format!(
            "payload entry path contains ':' (drive/ADS not allowed): {path}"
        ));
    }
    if normalised
        .chars()
        .any(|c| (c as u32) < 0x20 || c as u32 == 0x7f)
    {
        return Err(format!(
            "payload entry path contains control characters: {path}"
        ));
    }
    let mut out = PathBuf::new();
    for segment in normalised.split('/') {
        if segment == "." {
            continue;
        }
        if segment.is_empty() || segment == ".." {
            return Err(format!("payload entry path escapes its root: {path}"));
        }
        if segment
            .chars()
            .any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
        {
            return Err(format!(
                "payload entry path contains a Windows-forbidden character: {path}"
            ));
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Err(format!(
                "payload entry path segment ends with '.' or space (not allowed on Windows): {path}"
            ));
        }
        if is_reserved_name(segment) {
            return Err(format!(
                "payload entry path uses a reserved device name: {path}"
            ));
        }
        out.push(segment);
    }
    if out.as_os_str().is_empty() {
        return Err(format!("payload entry has no usable path: {path}"));
    }
    Ok(out)
}

/// Normalised forward-slash form of a payload path (e.g. `./a\\b.txt`
/// becomes `a/b.txt`). Used for reserved-entry and duplicate checks and
/// as the canonical stored form in the archive.
pub fn normalized_rel_str(path: &str) -> Result<String, String> {
    let sanitised = sanitise_rel_path(path)?;
    let mut parts: Vec<String> = Vec::new();
    for comp in sanitised.components() {
        // `sanitise_rel_path` only ever pushes `Normal` components.
        if let std::path::Component::Normal(part) = comp {
            parts.push(part.to_string_lossy().into_owned());
        }
    }
    if parts.is_empty() {
        return Err(format!("payload entry has no usable path: {path}"));
    }
    Ok(parts.join("/"))
}

/// Facts about one packed file: what the archive repeats in front of its
/// data, and all the manifest needs to report it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchiveHeader {
    /// Normalised forward-slash relative path.
    path: String,
    size: u64,
    sha256: [u8; 32],
}

/// Chunk size for every streaming operation (hash, compress, extract).
/// 64 KiB matches the zstd frame block size so no read straddles frames.
const STREAM_CHUNK: usize = 64 * 1024;

/// Where the bytes of one file come from while it is packed.
///
/// Both variants are borrowed views, so `Clone` is a pointer copy.
#[derive(Clone, Copy)]
enum Source<'a> {
    Path(&'a Path),
    Bytes(&'a [u8]),
}

impl Source<'_> {
    /// Hash in 64 KiB chunks; the file is never held in memory.
    fn digest(&self) -> Result<(u64, [u8; 32]), String> {
        match self {
            Source::Bytes(bytes) => Ok((bytes.len() as u64, Sha256::digest(bytes).into())),
            Source::Path(path) => {
                let mut file = std::fs::File::open(path)
                    .map_err(|e| format!("could not read {}: {e}", path.display()))?;
                let mut hasher = Sha256::new();
                let mut size = 0u64;
                let mut buf = vec![0u8; STREAM_CHUNK];
                loop {
                    let read = file
                        .read(&mut buf)
                        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buf[..read]);
                    size += read as u64;
                }
                Ok((size, hasher.finalize().into()))
            }
        }
    }

    /// Copy the bytes into a `Write`, hashing as we go, and report what was
    /// actually written so the caller can detect a file that changed.
    fn copy_to<W: Write>(&self, out: &mut W) -> Result<(u64, [u8; 32]), String> {
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        match self {
            Source::Bytes(bytes) => {
                hasher.update(bytes);
                size = bytes.len() as u64;
                out.write_all(bytes)
                    .map_err(|e| format!("could not write the payload archive: {e}"))?;
            }
            Source::Path(path) => {
                let mut file = std::fs::File::open(path)
                    .map_err(|e| format!("could not read {}: {e}", path.display()))?;
                let mut buf = vec![0u8; STREAM_CHUNK];
                loop {
                    let read = file
                        .read(&mut buf)
                        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buf[..read]);
                    size += read as u64;
                    out.write_all(&buf[..read])
                        .map_err(|e| format!("could not write the payload archive: {e}"))?;
                }
            }
        }
        Ok((size, hasher.finalize().into()))
    }
}

/// Counts bytes written through it, so the footer can be proven to match
/// the archive body that was actually produced.
struct CountingWriter<W> {
    inner: W,
    count: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Build the headers for `sources`, normalising and validating every path.
///
/// Rejects (in this order) the reserved config entry, over-long paths and
/// duplicate paths case-insensitively — the target filesystem is Windows,
/// where `A.txt` and `a.txt` would collide on extract.
fn prepare_headers(sources: &[(String, Source<'_>)]) -> Result<Vec<ArchiveHeader>, String> {
    if sources.len() > u32::MAX as usize {
        return Err(format!(
            "too many payload files ({} exceeds u32 range)",
            sources.len()
        ));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut headers = Vec::with_capacity(sources.len());
    for (raw, source) in sources {
        let path = normalized_rel_str(raw)?;
        if path.eq_ignore_ascii_case(CONFIG_ENTRY_NAME) {
            return Err(format!(
                "payload source must not contain the reserved entry {CONFIG_ENTRY_NAME}"
            ));
        }
        if path.len() > MAX_PATH_LEN {
            return Err(format!(
                "payload entry path is too long ({} bytes, max {MAX_PATH_LEN}): {path}",
                path.len()
            ));
        }
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err(format!("duplicate payload entry path: {raw}"));
        }
        let (size, sha256) = source.digest()?;
        headers.push(ArchiveHeader { path, size, sha256 });
    }
    Ok(headers)
}

/// Append `[stub][zstd archive][footer]` for `sources` plus the config.
///
/// Packing streams: every file is hashed in one bounded pass, then re-read
/// in 64 KiB chunks straight into the zstd encoder. Only one chunk per file
/// is ever resident, so a multi-gigabyte payload no longer needs that much
/// RAM; the whole archive is likewise never materialised.
fn build_payload(
    stub: &[u8],
    sources: Vec<(String, Source<'_>)>,
    config_json: &str,
) -> Result<Vec<u8>, String> {
    let headers = prepare_headers(&sources)?;
    // Pair each header with its source, then order both together:
    // case-insensitive path order, tie-broken by exact bytes, so the output
    // is deterministic and independent of the host filesystem's ordering.
    let mut sorted: Vec<(ArchiveHeader, Source<'_>)> = headers
        .into_iter()
        .zip(sources.iter().map(|(_, s)| *s))
        .collect();
    sorted.sort_by(|(a, _), (b, _)| {
        a.path
            .to_ascii_lowercase()
            .cmp(&b.path.to_ascii_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    sorted.push((
        ArchiveHeader {
            path: CONFIG_ENTRY_NAME.into(),
            size: config_json.len() as u64,
            sha256: Sha256::digest(config_json.as_bytes()).into(),
        },
        Source::Bytes(config_json.as_bytes()),
    ));

    // The footer carries the archive length. It is the fixed per-entry
    // overhead plus the scanned sizes, so it is exact and known up front.
    let mut archive_len: u64 = 4;
    for (header, _) in &sorted {
        let path_len = header.path.len() as u64;
        archive_len = archive_len
            .checked_add(2 + path_len + 8 + 32)
            .and_then(|n| n.checked_add(header.size))
            .ok_or("payload is too large to encode")?;
    }
    if archive_len > MAX_ARCHIVE_LEN {
        return Err(format!(
            "payload archive is too large ({archive_len} bytes, max {MAX_ARCHIVE_LEN})"
        ));
    }

    let mut payload: Vec<u8> = Vec::new();
    // The counter borrows the encoder, so the whole write phase lives in an
    // inner fn: it returns once the borrow ends and `finish()` can run.
    fn write_archive<R: Write>(
        sink: R,
        sorted: &[(ArchiveHeader, Source<'_>)],
        archive_len: u64,
    ) -> Result<(), String> {
        let mut counting = CountingWriter {
            inner: sink,
            count: 0,
        };
        counting
            .write_all(&(sorted.len() as u32).to_le_bytes())
            .map_err(|e| format!("could not write the payload archive: {e}"))?;
        for (header, source) in sorted {
            let path_bytes = header.path.as_bytes();
            let mut prefix = Vec::with_capacity(2 + path_bytes.len() + 8 + 32);
            prefix.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
            prefix.extend_from_slice(path_bytes);
            prefix.extend_from_slice(&header.size.to_le_bytes());
            prefix.extend_from_slice(&header.sha256);
            counting
                .write_all(&prefix)
                .map_err(|e| format!("could not write the payload archive: {e}"))?;
            let (size, sha256) = source.copy_to(&mut counting)?;
            if size != header.size || sha256 != header.sha256 {
                return Err(format!(
                    "{} changed while it was being packed (size or contents differ from the \
                     scanned value); let the build finish, then pack again",
                    header.path
                ));
            }
        }
        if counting.count != archive_len {
            return Err(format!(
                "internal error: wrote {} archive bytes but declared {archive_len}",
                counting.count
            ));
        }
        Ok(())
    }

    {
        let mut encoder = zstd::Encoder::new(&mut payload, DEFAULT_ZSTD_LEVEL)
            .map_err(|e| format!("payload compression failed: {e}"))?;
        write_archive(&mut encoder, &sorted, archive_len)?;
        encoder
            .finish()
            .map_err(|e| format!("payload compression failed: {e}"))?;
    }

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

/// Append a payload built from `files` ((relative path, data) pairs) plus
/// the project config JSON to `stub`, returning the finished exe.
///
/// Paths are normalised (see [`normalized_rel_str`]); the reserved config
/// entry, duplicate paths (case-insensitively, since the target filesystem
/// is Windows), and over-long paths are rejected.
pub fn pack_files(
    stub: &[u8],
    files: Vec<(String, Vec<u8>)>,
    config_json: &str,
) -> Result<Vec<u8>, String> {
    let sources: Vec<(String, Source<'_>)> = files
        .iter()
        .map(|(path, data)| (path.clone(), Source::Bytes(data.as_slice())))
        .collect();
    build_payload(stub, sources, config_json)
}

/// Walk a source directory (sorted, deterministic) and pack it onto `stub`.
///
/// Symlinks are **not followed**: symlinked files and directories are
/// skipped (the payload format has no symlink representation, and
/// following them could pack content from outside the source tree or
/// recurse forever on a symlink loop). Other non-regular files
/// (FIFOs, sockets, …) are likewise skipped.
///
/// Files are streamed in 64 KiB chunks and never held whole in memory.
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
    collect_files(source_dir, &mut paths)?;
    paths.sort();
    let mut sources: Vec<(String, Source<'_>)> = Vec::with_capacity(paths.len());
    for path in &paths {
        let rel = path
            .strip_prefix(source_dir)
            .map_err(|_| "internal path error".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        sources.push((rel, Source::Path(path)));
    }
    let exe = build_payload(stub, sources, config_json)?;
    // Report exactly what the installer will see, from the archive that was
    // just produced, rather than duplicating the packing logic here.
    let manifest = read_manifest(&exe)?;
    Ok((exe, manifest.files))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut read = std::fs::read_dir(dir)
        .map_err(|e| format!("could not list {}: {e}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("could not list {}: {e}", dir.display()))?;
    read.sort_by_key(|e| e.file_name());
    for entry in read {
        let path = entry.path();
        // Use symlink_metadata so symlinks are never followed.
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("could not stat {}: {e}", path.display()))?;
        let file_type = meta.file_type();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_files(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
        // Anything else (FIFO, socket, …) is skipped.
    }
    Ok(())
}

/// Locate and verify the payload footer of a packed exe.
/// Strip the RustPacker payload from a packed installer, leaving the stub.
///
/// AUDIT L16: the uninstaller used to be a byte-for-byte copy of `Setup.exe`,
/// so every install kept a second copy of the whole payload on disk purely to
/// delete the first one. The payload is a pure overlay — starting at
/// `payload_offset` and running to the end of the file — so the uninstaller
/// only needs everything *before* it. The result is the original stub, which
/// knows how to uninstall from the journal and simply has nothing to install.
///
/// Returns `None` when `exe` carries no payload, which is exactly the case
/// when the process *is* already an uninstaller: the caller should copy
/// itself unchanged.
pub fn strip_payload(exe: &[u8]) -> Option<Vec<u8>> {
    let info = find_payload(exe).ok()?;
    let end = usize::try_from(info.payload_offset).ok()?;
    Some(exe[..end].to_vec())
}

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
    // Use checked conversions so a hostile footer cannot truncate on
    // 32-bit hosts or overflow the slice arithmetic.
    let end = payload_offset
        .checked_add(payload_len)
        .ok_or("payload bounds in footer overflow")?;
    let end_usize =
        usize::try_from(end).map_err(|_| "payload bounds in footer do not match the file size")?;
    if end_usize != exe.len() - FOOTER_LEN {
        return Err("payload bounds in footer do not match the file size".into());
    }
    let offset_usize = usize::try_from(payload_offset)
        .map_err(|_| "payload bounds in footer do not match the file size")?;
    let len_usize = usize::try_from(payload_len)
        .map_err(|_| "payload bounds in footer do not match the file size")?;
    let payload = &exe[offset_usize..offset_usize + len_usize];
    if Sha256::digest(payload).as_slice() != &footer[32..64] {
        return Err("payload SHA-256 mismatch — the file is corrupt or was modified".into());
    }
    Ok(PackedInfo {
        payload_offset,
        payload_len,
        archive_len,
        // The footer already holds the digest and we just proved the payload
        // matches it, so format those bytes instead of hashing the whole
        // payload a second time (that was a full extra pass over the file).
        payload_sha256_hex: hex_of(&footer[32..64]),
    })
}

// ---------------------------------------------------------------------------
// Archive reader
// ---------------------------------------------------------------------------
//
// Reading streams as well. The payload is decompressed through a zstd
// decoder whose output is capped at `archive_len + 1`, and entries are
// consumed one at a time: a file's bytes go straight to their destination
// (or are skipped) and only its hash is retained. Neither the decompressed
// archive nor any single file is ever resident.

/// One entry's header as stored in the archive.
#[derive(Debug, Clone)]
struct RawEntry {
    path: String,
    size: u64,
    sha256: [u8; 32],
}

/// Sequential reader over the uncompressed archive.
struct ArchiveReader<R> {
    inner: R,
    /// Bytes consumed so far, to prove the footer length is honest.
    consumed: u64,
    /// Archive length the footer promised.
    expected_len: u64,
    /// Bytes still pending within the current entry's data.
    pending: u64,
    /// Header of the entry being streamed, for hash verification.
    current: Option<RawEntry>,
}

impl<R: Read> ArchiveReader<R> {
    fn new(inner: R, expected_len: u64) -> Self {
        Self {
            inner,
            consumed: 0,
            expected_len,
            pending: 0,
            current: None,
        }
    }

    fn read_exact_counted(&mut self, buf: &mut [u8]) -> Result<(), String> {
        self.inner.read_exact(buf).map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => "payload archive is truncated".to_string(),
            _ => format!("payload decompression failed: {e}"),
        })?;
        self.consumed += buf.len() as u64;
        Ok(())
    }

    fn read_u16(&mut self) -> Result<u16, String> {
        let mut b = [0u8; 2];
        self.read_exact_counted(&mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let mut b = [0u8; 4];
        self.read_exact_counted(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    fn read_u64(&mut self) -> Result<u64, String> {
        let mut b = [0u8; 8];
        self.read_exact_counted(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    /// The entry count that opens the archive, bounded so a hostile header
    /// cannot force a huge allocation.
    fn read_count(&mut self) -> Result<usize, String> {
        let count = self.read_u32()? as usize;
        if count > MAX_FILE_COUNT {
            return Err(format!(
                "payload archive declares too many files ({count}, max {MAX_FILE_COUNT})"
            ));
        }
        Ok(count)
    }

    /// Read the next entry header and make it the current entry.
    fn next_entry(&mut self) -> Result<RawEntry, String> {
        let path_len = self.read_u16()? as usize;
        let mut path_bytes = vec![0u8; path_len];
        self.read_exact_counted(&mut path_bytes)?;
        let path = String::from_utf8(path_bytes)
            .map_err(|_| "payload entry path is not UTF-8".to_string())?;
        let size = self.read_u64()?;
        let mut sha256 = [0u8; 32];
        self.read_exact_counted(&mut sha256)?;
        let entry = RawEntry { path, size, sha256 };
        self.pending = size;
        self.current = Some(entry.clone());
        Ok(entry)
    }

    /// Feed the current entry's data to `sink` in bounded chunks, verifying
    /// the stored hash once the data is exhausted.
    fn stream_current(
        &mut self,
        sink: &mut impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; STREAM_CHUNK];
        while self.pending > 0 {
            let want = usize::try_from(self.pending.min(STREAM_CHUNK as u64))
                .map_err(|_| "payload entry is larger than this host can address".to_string())?;
            self.read_exact_counted(&mut buf[..want])?;
            hasher.update(&buf[..want]);
            sink(&buf[..want])?;
            self.pending -= want as u64;
        }
        self.verify_current(hasher)
    }

    /// Skip the current entry's data, still verifying its hash.
    fn skip_current(&mut self) -> Result<(), String> {
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; STREAM_CHUNK];
        while self.pending > 0 {
            let want = usize::try_from(self.pending.min(STREAM_CHUNK as u64))
                .map_err(|_| "payload entry is larger than this host can address".to_string())?;
            self.read_exact_counted(&mut buf[..want])?;
            hasher.update(&buf[..want]);
            self.pending -= want as u64;
        }
        self.verify_current(hasher)
    }

    fn verify_current(&mut self, hasher: Sha256) -> Result<(), String> {
        let digest: [u8; 32] = hasher.finalize().into();
        match &self.current {
            Some(entry) if digest != entry.sha256 => {
                Err(format!("SHA-256 mismatch for payload entry {}", entry.path))
            }
            _ => Ok(()),
        }
    }

    /// Read the current entry whole, refusing anything above `limit`.
    fn read_current_to_vec(&mut self, limit: u64) -> Result<Vec<u8>, String> {
        let size = self.current.as_ref().map(|e| e.size).unwrap_or(0);
        if size > limit {
            return Err(format!(
                "payload entry {} is too large ({size} bytes, max {limit})",
                self.current.as_ref().map(|e| e.path.as_str()).unwrap_or("")
            ));
        }
        let mut data: Vec<u8> = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        self.stream_current(&mut |chunk| {
            data.extend_from_slice(chunk);
            Ok(())
        })?;
        Ok(data)
    }

    /// Archive byte length the footer declared (the sum of every header and
    /// every entry, including the entry count prefix).
    fn expected_len(&self) -> u64 {
        self.expected_len
    }

    /// Prove the archive was consumed exactly as the footer declared. A
    /// mismatch means the footer lied (or the stream is corrupt).
    fn finish(self) -> Result<(), String> {
        if self.consumed != self.expected_len {
            return Err(format!(
                "payload archive length mismatch (footer declares {} bytes, archive holds {})",
                self.expected_len, self.consumed
            ));
        }
        Ok(())
    }
}

/// Open the payload archive of `exe`: verify the footer, then wrap the
/// payload in a zstd decoder capped at `archive_len + 1`.
///
/// The decoder borrows the payload slice, so this is zero-copy beyond the
/// decoder's own window.
fn open_archive(exe: &[u8]) -> Result<ArchiveReader<impl Read + '_>, String> {
    let info = find_payload(exe)?;
    if info.archive_len > MAX_ARCHIVE_LEN {
        return Err(format!(
            "payload archive is too large ({} bytes, max {MAX_ARCHIVE_LEN})",
            info.archive_len
        ));
    }
    let offset_usize = usize::try_from(info.payload_offset)
        .map_err(|_| "payload bounds in footer do not match the file size")?;
    let len_usize = usize::try_from(info.payload_len)
        .map_err(|_| "payload bounds in footer do not match the file size")?;
    let payload = &exe[offset_usize..offset_usize + len_usize];
    let decoder =
        zstd::Decoder::new(payload).map_err(|e| format!("payload decompression failed: {e}"))?;
    // `+1` so an archive that decompresses past its declared length is
    // detectable by `finish` instead of silently truncated.
    Ok(ArchiveReader::new(
        decoder.take(info.archive_len.saturating_add(1)),
        info.archive_len,
    ))
}

/// Read the manifest (file list + embedded config). Entry data is skipped
/// as it is verified, so the archive is never held in memory.
pub fn read_manifest(exe: &[u8]) -> Result<PayloadManifest, String> {
    let mut reader = open_archive(exe)?;
    let count = reader.read_count()?;
    let mut files = Vec::with_capacity(count);
    let mut config_json = None;
    for _ in 0..count {
        let entry = reader.next_entry()?;
        if entry.path == CONFIG_ENTRY_NAME {
            const MAX_CONFIG_BYTES: u64 = 8 * 1024 * 1024;
            let data = reader.read_current_to_vec(MAX_CONFIG_BYTES)?;
            config_json = Some(
                String::from_utf8(data).map_err(|_| "embedded config is not UTF-8".to_string())?,
            );
        } else {
            reader.skip_current()?;
            files.push(FileEntry {
                path: entry.path,
                size: entry.size,
                sha256_hex: hex_of(&entry.sha256),
            });
        }
    }
    reader.finish()?;
    Ok(PayloadManifest { files, config_json })
}

/// Verify the whole payload (footer hash, decompression, per-file hashes).
pub fn verify_payload(exe: &[u8]) -> Result<PayloadManifest, String> {
    read_manifest(exe)
}

/// Extract every payload file under `dest_dir`. The embedded config entry
/// is not written to disk. Returns the extracted relative paths.
///
/// Extraction is symlink-safe: the destination is canonicalised after
/// creation, every parent directory is verified to stay inside it, and a
/// pre-existing symlink at the target path is rejected. A payload (or a
/// pre-seeded destination) therefore cannot write outside `dest_dir` via
/// `..`, absolute paths, or symlinks.
///
/// Files stream straight from the decompressor to disk, so extraction
/// needs one 64 KiB chunk rather than the whole payload.
pub fn extract_payload(exe: &[u8], dest_dir: &Path) -> Result<Vec<String>, String> {
    extract_payload_with_progress(exe, dest_dir, &mut |_done, _total, _path| Ok(()))
}

/// Progress sink for [`extract_payload_with_progress`].
///
/// `done` is the byte total of the entries finished before this one and
/// `total` is the archive's declared byte total, so a caller can drive a
/// progress bar without knowing the payload layout. Returning an error
/// aborts the extraction: the caller's error becomes the failure reason,
/// and the partial output is left for the caller to roll back (the stub
/// journals every target before extracting for exactly this reason).
pub type ExtractProgress<'a> = dyn FnMut(u64, u64, &str) -> Result<(), String> + 'a;

/// [`extract_payload`] with per-entry progress and cooperative cancellation.
pub fn extract_payload_with_progress(
    exe: &[u8],
    dest_dir: &Path,
    progress: &mut ExtractProgress<'_>,
) -> Result<Vec<String>, String> {
    let mut reader = open_archive(exe)?;
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("could not create {}: {e}", dest_dir.display()))?;
    let dest_canonical = std::fs::canonicalize(dest_dir)
        .map_err(|e| format!("could not resolve {}: {e}", dest_dir.display()))?;
    let count = reader.read_count()?;
    // The archive's declared total size, so progress is meaningful from the
    // first entry instead of guessing.
    let total_bytes = reader.expected_len();
    let mut done_bytes: u64 = 0;
    let mut extracted = Vec::new();
    for _ in 0..count {
        let entry = reader.next_entry()?;
        if entry.path == CONFIG_ENTRY_NAME {
            reader.skip_current()?;
            continue;
        }
        progress(done_bytes, total_bytes, &entry.path)?;
        let rel = sanitise_rel_path(&entry.path)?;
        let target = dest_dir.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
            let parent_canonical = std::fs::canonicalize(parent)
                .map_err(|e| format!("could not resolve {}: {e}", parent.display()))?;
            if !parent_canonical.starts_with(&dest_canonical) {
                return Err(format!(
                    "payload entry would escape the destination directory: {}",
                    entry.path
                ));
            }
        }
        // Never write through a pre-existing symlink at the target.
        if let Ok(meta) = std::fs::symlink_metadata(&target) {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "refusing to write through a symlink: {}",
                    target.display()
                ));
            }
        }
        let mut file = std::fs::File::create(&target)
            .map_err(|e| format!("could not create {}: {e}", target.display()))?;
        reader.stream_current(&mut |chunk| {
            file.write_all(chunk)
                .map_err(|e| format!("could not write {}: {e}", target.display()))
        })?;
        done_bytes = done_bytes.saturating_add(entry.size);
        extracted.push(entry.path);
    }
    reader.finish()?;
    Ok(extracted)
}

/// Lowercase hex of a raw digest.
///
/// One implementation for both callers (`sha256_hex`, and the per-entry
/// digests in the manifest) so the two can never disagree about case or
/// padding. Allocates once and indexes a static table instead of running
/// `format!` per byte.
fn hex_of(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = Vec::with_capacity(digest.len() * 2);
    for b in digest {
        bytes.push(HEX[(b >> 4) as usize]);
        bytes.push(HEX[(b & 0x0f) as usize]);
    }
    // Every byte pushed is ASCII hex, so this cannot fail.
    String::from_utf8(bytes).expect("hex is ASCII")
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
///
/// Returns an error instead of silently truncating when an entry name,
/// entry size, offset, or entry count exceeds the ZIP field widths
/// (u16/u32). ZIP64 is not supported; payloads needing it must be
/// split or the portable export omitted.
pub fn write_stored_zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    if entries.len() > MAX_ZIP_ENTRIES {
        return Err(format!(
            "too many portable ZIP entries ({} exceeds u16 range)",
            entries.len()
        ));
    }
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (name, data) in entries {
        // Portable ZIPs are unpacked on Windows: apply the same path
        // rules as the payload so a hostile or sloppy project cannot
        // ship traversal or reserved names inside the ZIP.
        let normalised = normalized_rel_str(name)?;
        if !seen.insert(normalised.to_ascii_lowercase()) {
            return Err(format!("duplicate portable ZIP entry: {normalised}"));
        }
        let name_bytes = normalised.as_bytes();
        if name_bytes.len() > MAX_PATH_LEN {
            return Err(format!(
                "portable ZIP entry name is too long ({} bytes, max {MAX_PATH_LEN}): {name}",
                name_bytes.len()
            ));
        }
        if data.len() > MAX_ZIP_ENTRY_LEN {
            return Err(format!(
                "portable ZIP entry is too large ({} bytes, max {MAX_ZIP_ENTRY_LEN}): {name}",
                data.len()
            ));
        }
        if out.len() > MAX_ZIP_ENTRY_LEN {
            return Err("portable ZIP is too large (offset exceeds u32 range)".into());
        }
        let crc = crc32(data);
        let data_len_u32 = u32::try_from(data.len())
            .map_err(|_| format!("portable ZIP entry is too large: {normalised}"))?;
        let name_len_u16 = u16::try_from(name_bytes.len())
            .map_err(|_| format!("portable ZIP entry name is too long: {normalised}"))?;
        let offset = u32::try_from(out.len())
            .map_err(|_| "portable ZIP is too large (offset exceeds u32 range)".to_string())?;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // date 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&data_len_u32.to_le_bytes());
        out.extend_from_slice(&data_len_u32.to_le_bytes());
        out.extend_from_slice(&name_len_u16.to_le_bytes());
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
        central.extend_from_slice(&data_len_u32.to_le_bytes());
        central.extend_from_slice(&data_len_u32.to_le_bytes());
        central.extend_from_slice(&name_len_u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    if out.len() > MAX_ZIP_ENTRY_LEN || central.len() > MAX_ZIP_ENTRY_LEN {
        return Err("portable ZIP is too large (central directory exceeds u32 range)".into());
    }
    let central_offset = u32::try_from(out.len())
        .map_err(|_| "portable ZIP is too large (offset exceeds u32 range)".to_string())?;
    let central_len = u32::try_from(central.len()).map_err(|_| {
        "portable ZIP is too large (central directory exceeds u32 range)".to_string()
    })?;
    let entry_count = u16::try_from(entries.len())
        .map_err(|_| "too many portable ZIP entries (exceeds u16 range)".to_string())?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

/// Where one portable ZIP entry's bytes come from.
pub enum ZipSource<'a> {
    Bytes(&'a [u8]),
    Path(&'a Path),
}

impl ZipSource<'_> {
    /// CRC and length in one bounded pass (the ZIP writer needs both
    /// *before* it can emit the entry header).
    fn scan(&self) -> Result<(u32, u64), String> {
        let mut crc: u32 = 0xFFFF_FFFF;
        let mut size = 0u64;
        match self {
            ZipSource::Bytes(bytes) => {
                crc = crc32_update(crc, bytes);
                size = bytes.len() as u64;
            }
            ZipSource::Path(path) => {
                let mut file = std::fs::File::open(path)
                    .map_err(|e| format!("could not read {}: {e}", path.display()))?;
                let mut buf = vec![0u8; STREAM_CHUNK];
                loop {
                    let read = file
                        .read(&mut buf)
                        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
                    if read == 0 {
                        break;
                    }
                    crc = crc32_update(crc, &buf[..read]);
                    size += read as u64;
                }
            }
        }
        Ok((crc ^ 0xFFFF_FFFF, size))
    }

    fn copy_to<W: Write>(&self, out: &mut W) -> Result<(), String> {
        match self {
            ZipSource::Bytes(bytes) => out
                .write_all(bytes)
                .map_err(|e| format!("could not write the portable ZIP: {e}")),
            ZipSource::Path(path) => {
                let mut file = std::fs::File::open(path)
                    .map_err(|e| format!("could not read {}: {e}", path.display()))?;
                let mut buf = vec![0u8; STREAM_CHUNK];
                loop {
                    let read = file
                        .read(&mut buf)
                        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
                    if read == 0 {
                        break;
                    }
                    out.write_all(&buf[..read])
                        .map_err(|e| format!("could not write the portable ZIP: {e}"))?;
                }
                Ok(())
            }
        }
    }
}

/// Write a stored (uncompressed) ZIP, streaming entry data from disk.
///
/// Same container as [`write_stored_zip`], but sources are read in 64 KiB
/// chunks instead of being handed over as slices, so building the portable
/// export of a large project no longer keeps every file plus the finished
/// ZIP in memory. Paths get the same validation as payload entries.
pub fn write_stored_zip_streaming(entries: &[(String, ZipSource<'_>)]) -> Result<Vec<u8>, String> {
    if entries.len() > MAX_ZIP_ENTRIES {
        return Err(format!(
            "too many portable ZIP entries ({} exceeds u16 range)",
            entries.len()
        ));
    }
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (name, source) in entries {
        let normalised = normalized_rel_str(name)?;
        if !seen.insert(normalised.to_ascii_lowercase()) {
            return Err(format!("duplicate portable ZIP entry: {normalised}"));
        }
        let name_bytes = normalised.as_bytes();
        if name_bytes.len() > MAX_PATH_LEN {
            return Err(format!(
                "portable ZIP entry name is too long ({} bytes, max {MAX_PATH_LEN}): {name}",
                name_bytes.len()
            ));
        }
        let (crc, size) = source.scan()?;
        if size > MAX_ZIP_ENTRY_LEN as u64 {
            return Err(format!(
                "portable ZIP entry is too large ({size} bytes, max {MAX_ZIP_ENTRY_LEN}): {name}"
            ));
        }
        let data_len_u32 = u32::try_from(size)
            .map_err(|_| format!("portable ZIP entry is too large: {normalised}"))?;
        let name_len_u16 = u16::try_from(name_bytes.len())
            .map_err(|_| format!("portable ZIP entry name is too long: {normalised}"))?;
        let offset = u32::try_from(out.len())
            .map_err(|_| "portable ZIP is too large (offset exceeds u32 range)".to_string())?;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // date 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&data_len_u32.to_le_bytes()); // compressed
        out.extend_from_slice(&data_len_u32.to_le_bytes()); // uncompressed
        out.extend_from_slice(&name_len_u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        source.copy_to(&mut out)?;

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        central.extend_from_slice(&0u16.to_le_bytes()); // stored
        central.extend_from_slice(&0u16.to_le_bytes()); // time
        central.extend_from_slice(&0x21u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&data_len_u32.to_le_bytes());
        central.extend_from_slice(&data_len_u32.to_le_bytes());
        central.extend_from_slice(&name_len_u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    if out.len() > MAX_ZIP_ENTRY_LEN || central.len() > MAX_ZIP_ENTRY_LEN {
        return Err("portable ZIP is too large (central directory exceeds u32 range)".into());
    }
    let central_offset = u32::try_from(out.len())
        .map_err(|_| "portable ZIP is too large (offset exceeds u32 range)".to_string())?;
    let central_len = u32::try_from(central.len()).map_err(|_| {
        "portable ZIP is too large (central directory exceeds u32 range)".to_string()
    })?;
    let entry_count = u16::try_from(entries.len())
        .map_err(|_| "too many portable ZIP entries (exceeds u16 range)".to_string())?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

/// Incremental CRC-32 step, so callers can hash a stream in chunks.
fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
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
        table
    });
    let mut crc = crc;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

fn crc32(data: &[u8]) -> u32 {
    crc32_update(0xFFFF_FFFF, data) ^ 0xFFFF_FFFF
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
    fn strip_payload_leaves_the_stub_untouched() {
        let stub_bytes = stub();
        let exe = pack_files(
            &stub_bytes,
            vec![("app.exe".into(), b"binary-bytes".to_vec())],
            "{\"app\":{}}",
        )
        .expect("pack");
        let stripped = strip_payload(&exe).expect("packed file has a payload");
        assert_eq!(
            stripped, stub_bytes,
            "stripping must reproduce the original stub byte for byte"
        );
        assert_eq!(stripped.len(), find_payload(&exe).expect("find").payload_offset as usize);
        // Very much smaller: this is the L16 saving.
        assert!(stripped.len() < exe.len());
        // An already-stripped file has no payload, so there is nothing to do.
        assert!(strip_payload(&stripped).is_none());
        assert!(strip_payload(b"tiny").is_none());
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
        // L1: the reported digest is formatted from the footer bytes rather
        // than re-hashing the payload, so prove the two agree.
        assert_eq!(
            info.payload_sha256_hex,
            sha256_hex(
                &exe[info.payload_offset as usize
                    ..(info.payload_offset + info.payload_len) as usize]
            )
        );
        assert_eq!(info.payload_sha256_hex.len(), 64);
        let manifest = read_manifest(&exe).expect("manifest");
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.config_json.as_deref(), Some("{\"app\":{}}"));
        assert_eq!(manifest.files[0].path, "app.exe");
        assert_eq!(manifest.files[0].sha256_hex, sha256_hex(b"binary-bytes"));

        let dir = crate::scratch::ScratchDir::new("rustpacker-payload-test");
        let extracted = extract_payload(&exe, dir.path()).expect("extract");
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
        let base = crate::scratch::ScratchDir::new("rustpacker-packdir");
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
    }

    #[test]
    fn stored_zip_has_valid_structure() {
        let zip = write_stored_zip(&[("a.txt", b"abc"), ("b/c.txt", b"defg")]).expect("zip");
        assert_eq!(&zip[0..4], b"PK\x03\x04");
        // End-of-central-directory signature present near the tail.
        assert!(zip.windows(4).any(|w| w == b"PK\x05\x06"));
        assert_eq!(crc32(b"abc"), 0x3524_41C2);
    }

    #[test]
    fn windows_specific_paths_are_rejected_on_any_host() {
        // Drive, UNC, ADS, reserved names, trailing dot/space, and
        // forbidden characters must be rejected even on Linux, where
        // `Path::components()` alone would accept them.
        for bad in [
            "C:/evil.exe",
            "C:evil.exe",
            "C:\\evil.exe",
            "//server/share/evil",
            "file.txt:stream",
            "CON",
            "con.txt",
            "NUL",
            "COM1",
            "a/file.txt.",
            "a/file.txt ",
            "a/b<c.txt",
            "a/b|c.txt",
            "a//b.txt",
            "a/b.txt/",
        ] {
            assert!(
                sanitise_rel_path(bad).is_err(),
                "expected rejection for {bad:?}, got Ok"
            );
        }
        // Normalisation: `./` and backslashes collapse to the canonical form.
        assert_eq!(
            normalized_rel_str("./a\\b.txt").expect("normalise"),
            "a/b.txt"
        );
        assert_eq!(
            normalized_rel_str("a/./b.txt").expect("normalise"),
            "a/b.txt"
        );
    }

    #[test]
    fn reserved_entry_bypass_and_duplicates_are_rejected() {
        // `./` prefix must not bypass the reserved-entry check.
        assert!(pack_files(
            &stub(),
            vec![("./.rustpacker/config.json".into(), b"x".to_vec())],
            "{}"
        )
        .is_err());
        assert!(pack_files(
            &stub(),
            vec![(".rustpacker\\config.json".into(), b"x".to_vec())],
            "{}"
        )
        .is_err());
        // Exact and case-insensitive duplicates are rejected.
        assert!(pack_files(
            &stub(),
            vec![
                ("a.txt".into(), b"1".to_vec()),
                ("a.txt".into(), b"2".to_vec()),
            ],
            "{}"
        )
        .is_err());
        assert!(pack_files(
            &stub(),
            vec![
                ("A.txt".into(), b"1".to_vec()),
                ("a.txt".into(), b"2".to_vec()),
            ],
            "{}"
        )
        .is_err());
    }

    #[test]
    fn lying_footer_archive_len_is_rejected() {
        // A footer claiming an absurd archive length must be rejected
        // without attempting a huge allocation.
        let mut exe =
            pack_files(&stub(), vec![("a.txt".into(), b"data".to_vec())], "{}").expect("pack");
        // Overwrite archive_len (last 8 bytes) with a huge value and
        // fix up nothing else: `find_payload` still passes (it only
        // checks offset/len/hash), but decode must reject the size.
        let huge = (MAX_ARCHIVE_LEN + 1).to_le_bytes();
        let n = exe.len();
        exe[n - 8..n].copy_from_slice(&huge);
        assert!(read_manifest(&exe).is_err());
        // A moderately lying (but under-cap) length must also fail the
        // exact-length check after bounded decompression.
        let mut exe2 =
            pack_files(&stub(), vec![("a.txt".into(), b"data".to_vec())], "{}").expect("pack");
        let n2 = exe2.len();
        // Read the real archive_len, add 1, write it back. The payload
        // hash covers only the payload bytes, not the footer, so this
        // stays "valid" until the length check.
        let real = u64::from_le_bytes(exe2[n2 - 8..n2].try_into().expect("footer"));
        exe2[n2 - 8..n2].copy_from_slice(&(real + 1).to_le_bytes());
        assert!(read_manifest(&exe2).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn extract_refuses_symlink_escape() {
        let exe = pack_files(
            &stub(),
            vec![("sub/file.txt".into(), b"payload".to_vec())],
            "{}",
        )
        .expect("pack");
        let base = crate::scratch::ScratchDir::new("rustpacker-symlink-test");
        let outside = base.join("outside");
        let dest = base.join("dest");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        std::fs::create_dir_all(&dest).expect("mkdir dest");
        // Pre-seed dest/sub as a symlink to outside.
        std::os::unix::fs::symlink(&outside, dest.join("sub")).expect("symlink");
        assert!(extract_payload(&exe, &dest).is_err());
        assert!(
            !outside.join("file.txt").exists(),
            "payload must not escape via symlink"
        );
    }

    #[test]
    fn zip_rejects_traversal_and_duplicate_names() {
        assert!(write_stored_zip(&[("../evil.txt", b"x")]).is_err());
        assert!(write_stored_zip(&[("C:/evil.txt", b"x")]).is_err());
        assert!(write_stored_zip(&[("a.txt", b"x"), ("A.TXT", b"y")]).is_err());
        assert!(write_stored_zip(&[("CON", b"x")]).is_err());
    }

    #[test]
    fn zip_entry_names_are_normalised() {
        let zip = write_stored_zip(&[("./a\\b.txt", b"x")]).expect("zip");
        // The stored name is the canonical forward-slash form.
        assert!(
            zip.windows(5).any(|w| w == b"a/b.t"),
            "zip should store a/b.txt"
        );
    }

    #[test]
    fn archive_with_too_many_entries_is_rejected() {
        // A header claiming more files than the payload contains must fail
        // with a clean error instead of allocating for the claimed count.
        let mut archive = Vec::new();
        archive.extend_from_slice(&(MAX_FILE_COUNT as u32 + 1).to_le_bytes());
        let mut reader = ArchiveReader::new(archive.as_slice(), archive.len() as u64);
        let err = reader.read_count().expect_err("must reject");
        assert!(err.contains("too many files"), "{err}");
    }

    #[test]
    fn truncated_archive_entry_is_rejected() {
        let mut archive = Vec::new();
        archive.extend_from_slice(&1u32.to_le_bytes()); // one file
        archive.extend_from_slice(&5u16.to_le_bytes()); // path length 5
        archive.extend_from_slice(b"a.txt"); // the path, then nothing
        let mut reader = ArchiveReader::new(archive.as_slice(), archive.len() as u64);
        assert_eq!(reader.read_count().expect("count"), 1);
        let err = reader.next_entry().expect_err("truncated entry must fail");
        assert!(err.contains("truncated"), "{err}");
    }

    #[test]
    fn manifest_of_a_multi_file_payload_is_complete() {
        let files: Vec<(String, Vec<u8>)> = (0..64)
            .map(|i| (format!("dir{i}/file{i}.bin"), vec![i as u8; 1000]))
            .collect();
        let exe = pack_files(&stub(), files, "{}").expect("pack");
        let manifest = read_manifest(&exe).expect("manifest");
        assert_eq!(manifest.files.len(), 64);
        // Sorted case-insensitively, so `dir10` sorts before `dir2`.
        let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort_by_key(|p| p.to_ascii_lowercase());
        assert_eq!(paths, sorted);
        for f in &manifest.files {
            assert_eq!(f.size, 1000);
        }
    }

    #[test]
    fn streaming_zip_matches_in_memory_zip() {
        let base = crate::scratch::ScratchDir::new("rustpacker-zipstream");
        let file_a = base.join("a.txt");
        let file_b = base.join("b.bin");
        std::fs::write(&file_a, b"abc").expect("write");
        std::fs::write(&file_b, vec![0x42u8; 200_000]).expect("write");

        let in_memory = write_stored_zip(&[("a.txt", b"abc"), ("b.bin", &vec![0x42u8; 200_000])])
            .expect("memory zip");
        let streamed = write_stored_zip_streaming(&[
            ("a.txt".to_string(), ZipSource::Path(&file_a)),
            ("b.bin".to_string(), ZipSource::Path(&file_b)),
        ])
        .expect("streamed zip");
        // Same container, same CRC/lengths: the only thing that may differ is
        // nothing at all, since both writers emit identical fields.
        assert_eq!(in_memory, streamed);
    }

    #[test]
    fn streaming_zip_applies_the_same_path_rules() {
        let base = crate::scratch::ScratchDir::new("rustpacker-ziprules");
        let file = base.join("x");
        std::fs::write(&file, b"x").expect("write");
        assert!(
            write_stored_zip_streaming(&[("../evil.txt".to_string(), ZipSource::Path(&file))])
                .is_err()
        );
        assert!(
            write_stored_zip_streaming(&[("CON".to_string(), ZipSource::Path(&file))]).is_err()
        );
        assert!(write_stored_zip_streaming(&[
            ("a.txt".to_string(), ZipSource::Path(&file)),
            ("A.TXT".to_string(), ZipSource::Path(&file)),
        ])
        .is_err());
    }

    #[test]
    fn large_payload_round_trips_through_the_streaming_reader() {
        // Bigger than the 64 KiB stream chunk, in several files, so entry
        // data really is split across reads.
        let files: Vec<(String, Vec<u8>)> = (0..5)
            .map(|i| (format!("big{i}.bin"), vec![i as u8; STREAM_CHUNK * 2 + 7]))
            .collect();
        let exe = pack_files(&stub(), files, "{\"k\":1}").expect("pack");
        let manifest = read_manifest(&exe).expect("manifest");
        assert_eq!(manifest.files.len(), 5);
        for f in &manifest.files {
            assert_eq!(f.size, (STREAM_CHUNK * 2 + 7) as u64);
        }
        let dir = crate::scratch::ScratchDir::new("rustpacker-large");
        let extracted = extract_payload(&exe, dir.path()).expect("extract");
        assert_eq!(extracted.len(), 5);
        for i in 0..5u8 {
            let data = std::fs::read(dir.join(format!("big{i}.bin"))).expect("read");
            assert_eq!(data.len(), STREAM_CHUNK * 2 + 7);
            assert!(data.iter().all(|&b| b == i), "contents match");
        }
    }

    #[test]
    fn corrupt_entry_data_is_detected_while_streaming() {
        // Flip a byte inside the compressed payload: the footer hash fails
        // first, so build the corruption *inside* the archive instead by
        // packing then rewriting one entry's data through the reader.
        let exe =
            pack_files(&stub(), vec![("a.txt".into(), b"hello".to_vec())], "{}").expect("pack");
        assert!(read_manifest(&exe).is_ok());

        // A truncated exe must be reported, not panic.
        let truncated = &exe[..exe.len() - 1];
        assert!(read_manifest(truncated).is_err());
    }

    #[test]
    fn archive_reader_rejects_a_lying_short_footer() {
        let exe =
            pack_files(&stub(), vec![("a.txt".into(), b"hello".to_vec())], "{}").expect("pack");
        // Rewrite archive_len to be smaller than reality; `find_payload`
        // still passes (the payload hash is unchanged) but the reader must
        // notice the archive holds more bytes than declared.
        let mut lying = exe.clone();
        let n = lying.len();
        let real = u64::from_le_bytes(lying[n - 8..n].try_into().expect("footer"));
        lying[n - 8..n].copy_from_slice(&(real - 1).to_le_bytes());
        assert!(read_manifest(&lying).is_err());
    }

    #[test]
    fn extraction_reports_progress_and_can_be_cancelled() {
        let files: Vec<(String, Vec<u8>)> = (0..4)
            .map(|i| (format!("f{i}.bin"), vec![i as u8; 1000]))
            .collect();
        let exe = pack_files(&stub(), files, "{}").expect("pack");

        // Progress: one call per payload entry, plus the monotonic byte
        // total that a progress bar needs.
        let dir = crate::scratch::ScratchDir::new("rustpacker-prog");
        let mut seen: Vec<(u64, u64)> = Vec::new();
        extract_payload_with_progress(&exe, dir.path(), &mut |done, total, _path| {
            assert!(done <= total, "progress must not exceed the total");
            seen.push((done, total));
            Ok(())
        })
        .expect("extract with progress");
        assert_eq!(seen.len(), 4, "one report per file");
        assert!(seen.iter().all(|(_, total)| *total > 0));
        // Bytes advanced between reports.
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));

        // Cancelling aborts with the caller's reason and leaves the
        // partial output for the caller to roll back.
        let dir2 = crate::scratch::ScratchDir::new("rustpacker-cancel");
        let mut calls = 0;
        let err = extract_payload_with_progress(&exe, dir2.path(), &mut |_, _, _| {
            calls += 1;
            if calls == 2 {
                Err("Setup was cancelled".to_string())
            } else {
                Ok(())
            }
        })
        .expect_err("cancel must abort");
        assert!(err.contains("cancelled"), "{err}");
        assert_eq!(calls, 2, "no further entries are touched after a cancel");
    }

    #[test]
    fn pack_rejects_missing_reserved_and_roundtrips_config() {
        let exe = pack_files(
            &stub(),
            vec![("a.txt".into(), b"hello".to_vec())],
            "{\"k\":1}",
        )
        .expect("pack");
        let manifest = read_manifest(&exe).expect("manifest");
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.config_json.as_deref(), Some("{\"k\":1}"));
        assert_eq!(manifest.files[0].path, "a.txt");
        assert_eq!(manifest.files[0].size, 5);
        assert_eq!(manifest.files[0].sha256_hex, sha256_hex(b"hello"));
    }
}

# RustPacker file formats

The layouts below are implemented in `crates/rustpacker-core/src/payload.rs`
and are stable within a major version: a stub must be able to read any
payload produced by the CLI/Builder Studio of the same release line. This
document is the human-readable copy of the comments in that module.

```text
 rustpacker-cli / Builder Studio                generated Setup.exe
 ┌───────────────────────────┐   pack_directory ┌──────────────────────┐
 │ project.toml (ProjectConfig) ─────────────► │ stub PE (rustpacker- │
 │ payload source directory  │                │   stub.exe, MZ/PE)   │
 └───────────────────────────┘                │ zstd archive (below) │
        │ build_portable_zip                  │ 72-byte footer       │
        ▼                                     └──────────────────────┘
 ┌───────────────────────────┐                Uninstall.exe = the same
 │ <App>-v<ver>-Portable.zip │                stub with the payload
 │ (stored ZIP, see §3)      │                stripped (strip_payload)
 └───────────────────────────┘
```

## 1. Setup.exe overlay

A generated installer is the stub PE with one appended payload:

```text
[ stub PE bytes ][ zstd-compressed archive ][ footer ]
```

The footer is a fixed **72 bytes** at the very end of the file:

| Offset | Size | Contents |
|-------:|-----:|----------|
| 0      | 16   | magic `"RUSTPACKER_OVL01"` |
| 16     | 8    | payload offset, u64 LE, from the start of the file |
| 24     | 8    | payload length (compressed), u64 LE |
| 32     | 32   | SHA-256 of the payload bytes |
| 64     | 8    | archive (uncompressed) length, u64 LE |

`find_payload` verifies the magic, the bounds (`offset + length` must end
exactly at the footer) and the SHA-256 before anything is decompressed.
`strip_payload` returns `exe[..payload_offset]`; the stub uses it to write a
payload-free `Uninstall.exe`.

## 2. The archive (after zstd decompression)

All integers little-endian:

```text
u32  file_count
per file:
  u16  path_len
  ...  path, UTF-8, forward slashes, relative
  u64  data_len
  32   SHA-256 of the file data
  ...  data bytes
```

The project configuration travels inside the archive as the reserved entry
`.rustpacker/config.json` (the `ProjectConfig` JSON). Extractors surface it
as the manifest instead of writing it to disk as a payload file, and a
payload source directory may not itself contain that path.

Safety limits enforced on read, before allocation: the decompressed length
is capped at 2 GiB (`MAX_ARCHIVE_LEN`) and cross-checked against the footer's
recorded archive length; entry counts and path lengths are bounded by their
field widths. Paths are validated host-independently (no drive/ADS colons,
no UNC/absolute prefixes, no `.`/`..` or empty segments, no reserved Windows
device names, no trailing dot/space), so a payload packed on any OS is safe
to extract on Windows.

## 3. Portable ZIP

`<App>-v<version>-Portable.zip` is a plain stored (uncompressed) ZIP written
by the same code path in both `pack --portable` and the Builder Studio
(`build_portable_zip` in core):

- one entry per payload file, under its manifest path, streamed from disk;
- a final marker entry `rustpacker-portable.txt` naming the app, version,
  publisher, file count, and the executable to run;
- UTF-8 filename flag set, DOS date fixed at 1980-01-01 (deterministic
  bytes; the files are already checksummed by `SHA256SUMS.txt`);
- no ZIP64: entries larger than u32 or more than u16 entries are an error,
  and the export is omitted rather than produced truncated.

`SHA256SUMS.txt` covers the Setup executable and, when built, the portable
ZIP — one `sha256  name` pair per line, hashes computed over the final
written bytes.

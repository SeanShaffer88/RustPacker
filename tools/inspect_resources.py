#!/usr/bin/env python3
"""Inspect Windows resources, before or after the linker.

`build.rs` emits a `.res` file and hands it to the linker; nothing in this
repository can check that the linker kept it without a Windows binary. This
script closes most of that gap from any platform, in two modes:

  --res  Parse the `.res` container `build.rs` wrote, decoding each payload:
         the manifest as XML, `RT_VERSION` down to its string table, and the
         icon as a DIB (dimensions, bit depth, sizes).
  (none) Parse the resource directory of a built PE. That confirms the linker
         actually embedded all four resources, but the payloads are gone
         (they live in the data section, not the directory).

    python3 tools/inspect_resources.py --res some/rustpacker.res
    python3 tools/inspect_resources.py target/**/release/*.exe

Exit status is 1 if something is wrong, so CI can assert on it.
"""

import struct
import sys
import xml.etree.ElementTree as ET

# Resource types this project embeds (winnt.h ordinals).
TYPES = {
    3: "RT_ICON",
    14: "RT_GROUP_ICON",
    16: "RT_VERSION",
    24: "RT_MANIFEST",
}
REQUIRED = {"RT_ICON", "RT_GROUP_ICON", "RT_VERSION", "RT_MANIFEST"}

# ---------------------------------------------------------------------------
# .res container (flat records; cvtres turns these into the PE directory)
# ---------------------------------------------------------------------------
#   [u32 DataSize] [u32 HeaderSize] [type] [name] [pad] [u32 DataVersion]
#   [u16 MemoryFlags] [u16 language] [u32 version] [u32 characteristics]
#   payload [pad]
# Each of type/name is u16 0xFFFF + u16 ordinal, or a NUL-terminated UTF-16
# string. HeaderSize counts from the record start to the payload.
#
# A file starts with a 32-byte prologue — a record-shaped header (DataSize 0,
# HeaderSize 32) plus a 16-byte zero null entry — which every reader skips.
# HeaderSize must be >= 32 (LLVM's MIN_HEADER_SIZE); a smaller one means the
# file is malformed even if the rest parses.
MIN_HEADER_SIZE = 7 * 4 + 2 * 2
PROLOGUE_LEN = 32


def read_identifier(blob, pos):
    """Return (label, new_pos) for a resource type or name."""
    marker = struct.unpack_from("<H", blob, pos)[0]
    if marker == 0xFFFF:
        return struct.unpack_from("<H", blob, pos + 2)[0], pos + 4
    # A string identifier: UTF-16 until a NUL, then (usually) the length
    # prefix that rc.exe writes in front of it.
    end = pos
    while blob[end : end + 2] != b"\0\0":
        end += 2
    return blob[pos:end].decode("utf-16-le"), end + 2


def parse_res(blob):
    """Yield (type_name, id, language, payload) for each record."""
    if len(blob) < PROLOGUE_LEN:
        raise ValueError("file is shorter than the 32-byte prologue")
    magic_size, magic_header = struct.unpack_from("<II", blob, 0)
    if magic_size != 0 or magic_header != PROLOGUE_LEN:
        raise ValueError(
            f"bad magic header: DataSize {magic_size}, HeaderSize {magic_header}"
            " (want 0 and 32)"
        )
    if any(blob[8:PROLOGUE_LEN]):
        # Bytes 8..16 finish the magic header, 16..32 are the null entry.
        raise ValueError("prologue after the 8-byte header is not all zero")
    pos = PROLOGUE_LEN
    while pos < len(blob):
        data_size, header_size = struct.unpack_from("<II", blob, pos)
        if header_size < MIN_HEADER_SIZE:
            raise ValueError(
                f"record at {pos:#x}: HeaderSize {header_size} < {MIN_HEADER_SIZE}"
            )
        res_type, after_type = read_identifier(blob, pos + 8)
        res_id, after_id = read_identifier(blob, after_type)
        payload_at = pos + header_size
        if payload_at % 4:
            raise ValueError(f"record at {pos:#x}: payload is not 4-byte aligned")
        # Suffix layout, ending at the payload: DataVersion(4), MemoryFlags(2),
        # language(2), version(4), characteristics(4).
        language = struct.unpack_from("<H", blob, payload_at - 10)[0]
        payload = blob[payload_at : payload_at + data_size]
        if len(payload) != data_size:
            raise ValueError(f"record at {pos:#x}: payload runs past end of file")
        yield TYPES.get(res_type, str(res_type)), res_id, language, payload
        pos = payload_at + data_size
        pos += (4 - pos % 4) % 4


def dump_version(payload):
    """Print a VS_VERSIONINFO tree: key, type, value length, children."""

    def walk(pos, end, depth):
        while pos < end - 5:
            wlen, vlen, wtype = struct.unpack_from("<HHH", payload, pos)
            if wlen < 6 or pos + wlen > end:
                return
            stop = pos + 6
            while payload[stop : stop + 2] != b"\0\0":
                stop += 2
            key = payload[pos + 6 : stop].decode("utf-16-le")
            value_at = stop + 2
            if vlen and wtype == 1:
                raw = payload[value_at : value_at + vlen * 2]
                shown = raw.decode("utf-16-le").rstrip("\0")
            elif vlen:
                shown = f"<{vlen} bytes>"
            else:
                shown = ""
            print(f"      {'  ' * depth}{key}: {shown}")
            child_at = value_at + (vlen * 2 if wtype == 1 else vlen)
            child_at += (4 - child_at % 4) % 4
            walk(child_at, pos + wlen, depth + 1)
            pos += wlen

    walk(0, len(payload), 0)


def dump_dib(payload):
    (size, width, height, planes, bits, compression, image_size) = struct.unpack_from(
        "<IiiHHII", payload, 0
    )
    ok = (
        size == 40
        and height == 2 * width
        and bits == 32
        and compression == 0
        and image_size == len(payload) - 40
    )
    print(
        f"      DIB {width}x{height // 2}, {bits}bpp, "
        f"{image_size} bytes of pixels, header {size} bytes: {'OK' if ok else 'MALFORMED'}"
    )
    return ok


def inspect_res(path):
    blob = open(path, "rb").read()
    print(f"{path}: {len(blob)} bytes")
    found = set()
    ok = True
    for res_type, res_id, language, payload in parse_res(blob):
        print(f"  {res_type} id={res_id} lang={language:#06x} {len(payload)} bytes")
        found.add(res_type)
        if res_type == "RT_MANIFEST":
            root = ET.fromstring(payload.decode("utf-8"))
            levels = [e.get("level") for e in root.iter() if e.tag.endswith("requestedExecutionLevel")]
            print(f"      valid XML, assemblyManifest, execution level {levels}")
        elif res_type == "RT_VERSION":
            dump_version(payload)
        elif res_type == "RT_ICON":
            ok = dump_dib(payload) and ok
        elif res_type == "RT_GROUP_ICON":
            count = struct.unpack_from("<H", payload, 4)[0]
            size = struct.unpack_from("<I", payload, 6 + 8)[0]
            icon_id = struct.unpack_from("<H", payload, 6 + 12)[0]
            print(f"      {count} icon(s), group entry claims {size} bytes, points at id {icon_id}")
    missing = REQUIRED - found
    if missing:
        print(f"  MISSING: {', '.join(sorted(missing))}")
        ok = False
    return ok


# ---------------------------------------------------------------------------
# PE resource directory (what survived linking)
# ---------------------------------------------------------------------------


def sections(blob):
    pe = struct.unpack_from("<I", blob, 0x3C)[0]
    if blob[pe : pe + 4] != b"PE\0\0":
        raise ValueError("not a PE file")
    count = struct.unpack_from("<H", blob, pe + 6)[0]
    opt = pe + 24
    table = opt + struct.unpack_from("<H", blob, pe + 20)[0]
    out = []
    for i in range(count):
        off = table + i * 40
        vsize, va, rsize, rptr = struct.unpack_from("<IIII", blob, off + 8)
        out.append((va, max(vsize, rsize), rptr))
    return out, opt


def file_offset(secs, rva):
    for va, size, rptr in secs:
        if va <= rva < va + size:
            return rptr + (rva - va)
    raise ValueError(f"rva {rva:#x} is not inside any section")


def walk(blob, base, pos, prefix, found):
    named, ids = struct.unpack_from("<HH", blob, base + pos + 12)
    for i in range(named + ids):
        name, child = struct.unpack_from("<II", blob, base + pos + 16 + i * 8)
        label = name & 0x7FFFFFFF
        if not prefix:
            label = TYPES.get(label, str(label))
        if child & 0x80000000:
            walk(blob, base, child & 0x7FFFFFFF, prefix + [label], found)
        else:
            data_rva, data_size = struct.unpack_from("<II", blob, base + child)
            found.append((prefix + [label], data_rva, data_size))


def inspect_pe(path):
    blob = open(path, "rb").read()
    secs, opt = sections(blob)
    magic = struct.unpack_from("<H", blob, opt)[0]
    dd = opt + (112 if magic == 0x20B else 96)
    rva = struct.unpack_from("<I", blob, dd + 2 * 8)[0]
    print(f"{path}: resource directory rva {rva:#x}")
    if rva == 0:
        print("  NO RESOURCE TABLE")
        return False
    found = []
    walk(blob, file_offset(secs, rva), 0, [], found)
    for names, _, data_size in found:
        print(f"  {' / '.join(str(n) for n in names)}: {data_size} bytes")
    present = {str(n[0]) for n, _, _ in found}
    missing = REQUIRED - present
    if missing:
        print(f"  MISSING: {', '.join(sorted(missing))}")
        return False
    return True


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    ok = True
    if argv[0] == "--res":
        for path in argv[1:]:
            ok = inspect_res(path) and ok
    else:
        for path in argv:
            ok = inspect_pe(path) and ok
    print("RESULT:", "ok" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

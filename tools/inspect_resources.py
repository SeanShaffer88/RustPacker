#!/usr/bin/env python3
"""Print the resource table of a Windows PE binary.

`build.rs` emits a `.res` file and hands it to the linker; nothing in this
repository can check that the linker actually kept it, because that requires a
Windows binary. This script closes that gap from any platform: it parses the
PE resource directory of a built `.exe` and lists every type/id/language with
the size of its payload.

    python3 tools/inspect_resources.py target/x86_64-pc-windows-msvc/release/*.exe

Exit status is 1 if a binary has no table or a resource is missing, so it can
be used as an assertion in CI.
"""

import struct
import sys

# Resource types this project embeds (winnt.h ordinals).
TYPES = {
    3: "RT_ICON",
    14: "RT_GROUP_ICON",
    16: "RT_VERSION",
    24: "RT_MANIFEST",
}
REQUIRED = {"RT_ICON", "RT_GROUP_ICON", "RT_VERSION", "RT_MANIFEST"}


def sections(blob):
    """(virtual address, virtual size, raw pointer) for each PE section."""
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
    """Depth-first walk of IMAGE_RESOURCE_DIRECTORY, appending to `found`."""
    named, ids = struct.unpack_from("<HH", blob, base + pos + 12)
    for i in range(named + ids):
        name, child = struct.unpack_from("<II", blob, base + pos + 16 + i * 8)
        label = name & 0x7FFFFFFF
        if not prefix:
            label = TYPES.get(label, str(label))
        if child & 0x80000000:
            walk(blob, base, child & 0x7FFFFFFF, prefix + [label], found)
        else:
            entry = base + child
            data_rva, data_size = struct.unpack_from("<II", blob, entry)
            found.append((prefix + [label], data_rva, data_size))


def inspect(path):
    blob = open(path, "rb").read()
    secs, opt = sections(blob)
    magic = struct.unpack_from("<H", blob, opt)[0]
    # The resource directory is data-directory entry 2, at the same offset in
    # the PE32 and PE32+ optional headers.
    dd = opt + (112 if magic == 0x20B else 96)
    rva, _ = struct.unpack_from("<II", blob, dd + 2 * 8)
    print(f"{path}: resource directory rva {rva:#x}")
    if rva == 0:
        print("  NO RESOURCE TABLE")
        return set()

    found = []
    walk(blob, file_offset(secs, rva), 0, [], found)
    for names, data_rva, data_size in found:
        # Only the payload size is interesting here; type/id/lang identify it.
        print(f"  {' / '.join(str(n) for n in names)}: {data_size} bytes")
    return {str(names[0]) for names, _, _ in found}


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    status = 0
    for path in argv:
        present = inspect(path)
        missing = REQUIRED - present
        if missing:
            print(f"  MISSING: {', '.join(sorted(missing))}")
            status = 1
        else:
            print("  all four resources present")
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

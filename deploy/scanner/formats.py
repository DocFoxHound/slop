"""Opening packed files for the scanner (scan.py): archives, installers and app bundles.

Files are recognised by their first bytes, not their names. Each opener writes what it finds into
a new folder and never runs anything from the file. `open_up` answers what happened:

  - None: not something we open;
  - Opened(): its contents are in the folder;
  - Failed(why): it is that format, but couldn't be opened (damaged, encrypted, a version we
    don't know).

Our own readers (Godot packs, Electron's .asar, PyInstaller bundles) only write inside the
folder: names that are absolute or climb out of it are skipped.
"""

import bz2
import gzip
import hashlib
import json
import lzma
import os
import struct
import subprocess
import zlib

try:
    import pefile
except ImportError:  # the scanner image has it; this keeps the module importable elsewhere
    pefile = None

TAR = ["bsdtar", "-x", "--no-same-owner", "--no-same-permissions"]
# Most bytes a single file inside a bundle may be, when we unpack it ourselves.
MAX_MEMBER = 2 * 1024**3


class Opened:
    pass


class Failed:
    def __init__(self, why):
        self.why = why


def head(path, n=512):
    try:
        with open(path, "rb") as fh:
            return fh.read(n)
    except OSError:
        return b""


def kind_of(path):
    """What a file is, by its first bytes."""
    b = head(path, 1024)
    name = path.lower()
    if b[:4] in (b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"):
        return "zip"
    if len(b) > 262 and b[257:262] == b"ustar":
        return "tar"
    if b[:2] == b"\x1f\x8b":
        return "gzip"
    if b[:6] == b"\xfd7zXZ\x00":
        return "xz"
    if b[:3] == b"BZh":
        return "bzip2"
    if b[:4] == b"\x28\xb5\x2f\xfd":
        return "zstd"
    if b[:6] == b"7z\xbc\xaf\x27\x1c":
        return "7z"
    if b[:6] == b"Rar!\x1a\x07":
        return "rar"
    if b[:4] == b"MSCF":
        return "cab"
    if b[:8] == b"!<arch>\n":
        return "ar"
    if b[:4] == b"\xed\xab\xee\xdb":
        return "rpm"
    if b[:6] in (b"070701", b"070702", b"070707"):
        return "cpio"
    if b[:4] == b"hsqs":
        return "squashfs"
    if b[:8] == b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" and name.endswith((".msi", ".msp", ".msm")):
        return "msi"
    if b[:4] == b"GDPC":
        return "pck"
    if name.endswith(".asar") and b[:4] == b"\x04\x00\x00\x00":
        return "asar"
    if b[:2] == b"MZ":
        return "pe"
    if b[:4] == b"\x7fELF":
        return "elf"
    return None


def run(args, timeout=300):
    return subprocess.run(args, stdin=subprocess.DEVNULL, capture_output=True, timeout=timeout)


def safe_path(dest, name):
    """Where a member called `name` goes inside dest, or None if it would land outside."""
    name = name.replace("\\", "/")
    for prefix in ("res://", "./"):
        if name.startswith(prefix):
            name = name[len(prefix):]
    parts = [p for p in name.split("/") if p not in ("", ".")]
    if not parts or ".." in parts or any("\x00" in p for p in parts):
        return None
    return os.path.join(dest, *parts)


def write_member(dest, name, data):
    out = safe_path(dest, name)
    if out is None or os.path.exists(out):
        return False
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "wb") as fh:
        fh.write(data)
    return True


def copy_range(src, start, size, out):
    with open(out, "wb") as fh:
        src.seek(start)
        left = size
        while left > 0:
            chunk = src.read(min(left, 1024 * 1024))
            if not chunk:
                break
            fh.write(chunk)
            left -= len(chunk)


# ---- archives the usual tools read -------------------------------------------------------------

def bsdtar(path, dest):
    return run(TAR + ["-C", dest, "-f", path]).returncode == 0


def seven_zip(path, dest):
    # 7-Zip refuses links that point outside the folder by itself. A made-up password makes an
    # encrypted archive fail instead of asking for one.
    return run(["7z", "x", "-y", "-pnone", "-bd", "-o" + dest, path], timeout=600).returncode == 0


def rpm(path, dest):
    # rpm2cpio reads every RPM format; bsdtar alone can't read RPM 6 packages (Fedora 43's
    # rpmbuild can make them), whose payload lists files by number, not by name.
    with open(path, "rb") as fh:
        conv = subprocess.Popen(["rpm2cpio", "-"], stdin=fh, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        r = subprocess.run(TAR + ["-C", dest, "-f", "-"], stdin=conv.stdout, capture_output=True, timeout=300)
        conv.stdout.close()
        return conv.wait(timeout=60) == 0 and r.returncode == 0


def single_stream(path, dest, kind):
    """A compressed file that isn't a tar archive: decompress it to one file."""
    opener = {"gzip": gzip.open, "xz": lzma.open, "bzip2": bz2.open}.get(kind)
    if opener is None:
        return False
    base = os.path.basename(path)
    for ext in (".gz", ".xz", ".bz2", ".tgz"):
        if base.lower().endswith(ext):
            base = base[: -len(ext)] or "file"
            break
    else:
        base += ".out"
    try:
        with opener(path, "rb") as src, open(os.path.join(dest, base), "wb") as out:
            left = MAX_MEMBER
            while left > 0:
                chunk = src.read(1024 * 1024)
                if not chunk:
                    return True
                out.write(chunk)
                left -= len(chunk)
        return False
    except (OSError, EOFError, lzma.LZMAError, zlib.error):
        return False


# ---- Godot packs (.pck, and packs inside a game's program) ---------------------------------------

def godot_pck(path, dest, start=0):
    """Unpacks a Godot pack starting at byte `start` of the file.

    Formats 1 (Godot 3) and 2 (Godot 4.0 to 4.3) keep the file list after the header; 3 and 4
    (Godot 4.4 and later) say where it is. Newer formats are read like 4, and only trusted if the
    MD5 Godot recorded for a file matches.
    """
    with open(path, "rb") as f:
        size = os.fstat(f.fileno()).st_size
        f.seek(start)
        if f.read(4) != b"GDPC":
            return Failed("not a Godot pack")
        version, _major, _minor, _patch = struct.unpack("<4I", f.read(16))
        flags, file_base = 0, 0
        if version >= 2:
            flags, file_base = struct.unpack("<IQ", f.read(12))
        if flags & 1:
            return Failed("an encrypted Godot pack")
        if version >= 3:
            (dir_offset,) = struct.unpack("<Q", f.read(8))
            f.seek(start + dir_offset)
        else:
            f.read(64)  # reserved
        (count,) = struct.unpack("<I", f.read(4))
        if count > 1_000_000:
            return Failed("a damaged Godot pack")
        entries = []
        for _ in range(count):
            (name_len,) = struct.unpack("<I", f.read(4))
            if name_len > 4096:
                return Failed("a damaged Godot pack")
            name = f.read(name_len).split(b"\x00", 1)[0].decode("utf-8", "replace")
            ofs, length = struct.unpack("<QQ", f.read(16))
            md5 = f.read(16)
            file_flags = struct.unpack("<I", f.read(4))[0] if version >= 2 else 0
            entries.append((name, ofs, length, md5, file_flags))
        # Where offsets count from moved between Godot versions: take the base under which the
        # first file's MD5 (recorded by Godot when it made the pack) matches.
        bases = [file_base, file_base + start, start, 0]
        base = None
        probe = next((e for e in entries if e[2] and not e[4] & 1 and e[3] != bytes(16)), None)
        if probe is None and version > 4:
            return Failed(f"a Godot pack in a format we don't read yet ({version})")
        if probe is None:
            base = next((b for b in bases if all(b + e[1] + e[2] <= size for e in entries)), None)
        else:
            for b in bases:
                if b + probe[1] + probe[2] <= size:
                    f.seek(b + probe[1])
                    if probe[2] <= MAX_MEMBER and hashlib.md5(f.read(probe[2])).digest() == probe[3]:
                        base = b
                        break
        if base is None:
            return Failed("a Godot pack we couldn't read")
        encrypted = 0
        for name, ofs, length, _md5, file_flags in entries:
            if file_flags & 1:
                encrypted += 1
                continue
            if file_flags & 2 or length > MAX_MEMBER or base + ofs + length > size:
                continue
            out = safe_path(dest, name)
            if out is None or os.path.exists(out):
                continue
            os.makedirs(os.path.dirname(out), exist_ok=True)
            copy_range(f, base + ofs, length, out)
        if encrypted:
            return Failed(f"{encrypted} encrypted files in a Godot pack")
    return Opened()


def godot_embedded(path, size):
    """Where a pack added to the end of a Godot game's program starts, if there is one."""
    if size < 16:
        return None
    with open(path, "rb") as f:
        f.seek(size - 12)
        tail = f.read(12)
        if tail[8:] != b"GDPC":
            return None
        (pck_size,) = struct.unpack("<Q", tail[:8])
        start = size - 12 - pck_size
        if start < 0:
            return None
        f.seek(start)
        return start if f.read(4) == b"GDPC" else None


# ---- Electron's app.asar -------------------------------------------------------------------------

def asar(path, dest):
    with open(path, "rb") as f:
        size = os.fstat(f.fileno()).st_size
        _, header_size, _, json_len = struct.unpack("<4I", f.read(16))
        if json_len > header_size or header_size > 256 * 1024 * 1024:
            return Failed("a damaged .asar")
        try:
            tree = json.loads(f.read(json_len))
        except ValueError:
            return Failed("a damaged .asar")
        data = 8 + header_size

        def walk(node, prefix):
            for name, entry in (node.get("files") or {}).items():
                rel = prefix + "/" + name if prefix else name
                if "files" in entry:
                    walk(entry, rel)
                elif "offset" in entry and not entry.get("unpacked") and "link" not in entry:
                    ofs, length = int(entry["offset"]), int(entry.get("size", 0))
                    out = safe_path(dest, rel)
                    if out is None or os.path.exists(out) or length > MAX_MEMBER or data + ofs + length > size:
                        continue
                    os.makedirs(os.path.dirname(out), exist_ok=True)
                    copy_range(f, data + ofs, length, out)

        try:
            walk(tree, "")
        except (ValueError, TypeError, AttributeError, RecursionError):
            return Failed("a damaged .asar")
    return Opened()


# ---- PyInstaller bundles ---------------------------------------------------------------------------

MEI = b"MEI\x0c\x0b\x0a\x0b\x0e"


def pyinstaller_cookie(path, size):
    with open(path, "rb") as f:
        start = max(0, size - 128 * 1024)
        f.seek(start)
        tail = f.read()
    at = tail.rfind(MEI)
    return None if at < 0 else start + at


def pyinstaller(path, dest, cookie):
    with open(path, "rb") as f:
        f.seek(cookie)
        raw = f.read(88)
        new = len(raw) >= 88 and b"python" in raw[24:88].lower()
        cookie_size = 88 if new else 24
        _, package_len, toc, toc_len, _ = struct.unpack("!8sIIii", raw[:24])
        start = cookie + cookie_size - package_len
        if start < 0 or toc_len > 64 * 1024 * 1024:
            return Failed("a PyInstaller bundle we couldn't read")
        f.seek(start + toc)
        table = f.read(toc_len)
        pos = 0
        while pos + 18 <= len(table):
            entry_size, entry_pos, packed, unpacked, compressed, _kind = struct.unpack("!iIIIBc", table[pos:pos + 18])
            if entry_size < 18:
                return Failed("a PyInstaller bundle we couldn't read")
            name = table[pos + 18:pos + entry_size].split(b"\x00", 1)[0].decode("utf-8", "replace")
            pos += entry_size
            if packed > MAX_MEMBER or unpacked > MAX_MEMBER:
                continue
            f.seek(start + entry_pos)
            data = f.read(packed)
            if compressed:
                try:
                    data = zlib.decompress(data)
                except zlib.error:
                    continue
            write_member(dest, name or "member", data)
    return Opened()


# ---- programs: installers, self-extracting archives, packers ---------------------------------------

def pe_overlay(path, size):
    """Where data added after a Windows program's own parts starts and ends (not its signature)."""
    if pefile is None:
        return None
    try:
        pe = pefile.PE(path, fast_load=True)
    except Exception:
        return None
    try:
        end = max((s.PointerToRawData + s.SizeOfRawData for s in pe.sections), default=0)
        stop = size
        security = pe.OPTIONAL_HEADER.DATA_DIRECTORY[4] if len(pe.OPTIONAL_HEADER.DATA_DIRECTORY) > 4 else None
        if security and security.VirtualAddress >= end and security.Size:
            stop = security.VirtualAddress
        packed = any(s.Name.rstrip(b"\x00") in (b"UPX0", b"UPX1") for s in pe.sections)
        return end, stop, packed
    finally:
        pe.close()


def nsis_at(path, start, stop):
    """NSIS installers put their data after the program, at a 512-byte boundary."""
    with open(path, "rb") as f:
        pos = start - start % 512
        while pos < min(stop, start + 1024 * 1024):
            f.seek(pos + 4)
            if f.read(16) == b"\xef\xbe\xad\xdeNullsoftInst":
                return True
            pos += 512
    return False


def contains(path, marker, start=0, limit=1024 * 1024):
    with open(path, "rb") as f:
        f.seek(start)
        tail = b""
        read = 0
        while read < limit:
            chunk = f.read(1024 * 1024)
            if not chunk:
                return False
            if marker in tail + chunk:
                return True
            tail = chunk[-len(marker):]
            read += len(chunk)
    return False


def elf_end(path):
    """Where an ELF program's own parts end (an AppImage keeps its files after that)."""
    b = head(path, 64)
    if len(b) < 64:
        return None
    endian = "<" if b[5] == 1 else ">"
    if b[4] == 2:
        shoff, = struct.unpack(endian + "Q", b[40:48])
        shentsize, shnum = struct.unpack(endian + "HH", b[58:62])
    else:
        shoff, = struct.unpack(endian + "I", b[32:36])
        shentsize, shnum = struct.unpack(endian + "HH", b[46:50])
    return shoff + shentsize * shnum


def upx(path, dest):
    out = os.path.join(dest, os.path.basename(path))
    return run(["upx", "-d", "-q", "-o", out, path]).returncode == 0


def program(path, dest, kind):
    size = os.path.getsize(path)
    cookie = pyinstaller_cookie(path, size)
    if cookie is not None:
        return pyinstaller(path, dest, cookie)
    pck = godot_embedded(path, size)
    if pck is not None:
        return godot_pck(path, dest, pck)
    if kind == "elf":
        if head(path, 11)[8:11] == b"AI\x02":
            end = elf_end(path)
            ok = end and run(["unsquashfs", "-q", "-n", "-no-xattrs", "-o", str(end), "-d", os.path.join(dest, "AppImage"), path]).returncode == 0
            return Opened() if ok else Failed("an AppImage we couldn't open")
        if b"UPX!" in head(path, 4096):
            return Opened() if upx(path, dest) else None
        return None
    overlay = pe_overlay(path, size)
    if overlay is None:
        return None
    start, stop, packed = overlay
    if stop - start >= 4096:
        if nsis_at(path, start, stop):
            return Opened() if seven_zip(path, dest) else Failed("an NSIS installer we couldn't open")
        with open(path, "rb") as f:
            f.seek(start)
            magic = f.read(8)
        if magic[:4] in (b"PK\x03\x04", b"MSCF") or magic[:6] in (b"7z\xbc\xaf\x27\x1c", b"Rar!\x1a\x07"):
            return Opened() if seven_zip(path, dest) else Failed("a self-extracting archive we couldn't open")
        if contains(path, b"Inno Setup Setup Data (", start):
            ok = run(["innoextract", "-e", "-s", "-q", "-d", dest, path], timeout=600).returncode == 0
            return Opened() if ok else Failed("an Inno Setup installer we couldn't open")
    if packed:
        return Opened() if upx(path, dest) else None
    return None


# ---- the dispatcher ------------------------------------------------------------------------------------

def open_up(path, dest, kind=None):
    kind = kind or kind_of(path)
    if kind is None:
        return None
    os.makedirs(dest, exist_ok=True)
    try:
        if kind == "rpm":
            return Opened() if rpm(path, dest) else Failed("damaged, encrypted or too large")
        if kind in ("zip", "tar", "ar", "cab", "cpio", "gzip", "xz", "bzip2", "zstd"):
            if bsdtar(path, dest):
                return Opened()
            if kind in ("gzip", "xz", "bzip2") and single_stream(path, dest, kind):
                return Opened()
            return Failed("damaged, encrypted or too large")
        if kind in ("7z", "rar", "msi"):
            return Opened() if bsdtar(path, dest) or seven_zip(path, dest) else Failed("damaged, encrypted or too large")
        if kind == "squashfs":
            ok = run(["unsquashfs", "-q", "-n", "-no-xattrs", "-d", os.path.join(dest, "files"), path]).returncode == 0
            return Opened() if ok else Failed("damaged or too large")
        if kind == "pck":
            return godot_pck(path, dest)
        if kind == "asar":
            return asar(path, dest)
        if kind in ("pe", "elf"):
            return program(path, dest, kind)
    except subprocess.TimeoutExpired:
        return Failed("took too long to open")
    except (OSError, struct.error, ValueError, MemoryError) as e:
        return Failed(f"couldn't be read ({type(e).__name__})")
    return None

"""Fingerprints of a release's programs, and matching them against newer ClamAV signatures.

Most of ClamAV's signatures are fingerprints: of a whole file (MD5, SHA-1 or SHA-256 with its
size: .hdb, .hsb) or of one section of a Windows program (.mdb, .msb). So the scanner records
those fingerprints for every program and script in a release (`record`), SLAPorSLOP keeps them,
and `match` checks them against today's signatures without downloading anything again: a match
sends that release for a full re-scan (crates/slop-api/src/rescan.rs).

`match` reads JSON lines on stdin, {"id", "kind": "file"|"section", "size", "md5", "sha1",
"sha256"}, and prints {"engines": {...}, "matches": [{"id", "name"}]}. False-positive lists
(.fp, .sfp) and ignored signatures (.ign2) are honoured, as clamscan would.
"""

import hashlib
import json
import os
import sys
import tarfile

try:
    import pefile
except ImportError:
    pefile = None

PROGRAM_MAGIC = (b"MZ", b"\x7fELF", b"\xfe\xed\xfa\xce", b"\xfe\xed\xfa\xcf", b"\xce\xfa\xed\xfe",
                 b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"dex\n", b"\xd0\xcf\x11\xe0")
# By name: scripts, and programs whatever their first bytes (an old .com has none to go by).
SCRIPTS = (".js", ".jse", ".vbs", ".vbe", ".ps1", ".psm1", ".bat", ".cmd", ".hta", ".wsf", ".py",
           ".sh", ".lua", ".jar", ".lnk", ".exe", ".com", ".scr", ".cpl", ".sys", ".dll", ".so", ".dylib")
MAX_FILES = 3000
MAX_SECTIONS = 32
MAX_HASHED = 2 * 1024**3


def program_like(path):
    if path.lower().endswith(SCRIPTS):
        return True
    try:
        with open(path, "rb") as fh:
            magic = fh.read(4)
    except OSError:
        return False
    return magic.startswith(PROGRAM_MAGIC)


def digests(data_iter):
    h = [hashlib.md5(), hashlib.sha1(), hashlib.sha256()]
    size = 0
    for chunk in data_iter:
        size += len(chunk)
        for x in h:
            x.update(chunk)
    return {"size": size, "md5": h[0].hexdigest(), "sha1": h[1].hexdigest(), "sha256": h[2].hexdigest()}


def chunks(path):
    with open(path, "rb") as fh:
        while True:
            c = fh.read(1024 * 1024)
            if not c:
                return
            yield c


def sections(path):
    """Fingerprints of a Windows program's sections, as ClamAV's .mdb/.msb signatures take them."""
    if pefile is None:
        return []
    try:
        pe = pefile.PE(path, fast_load=True)
    except Exception:
        return []
    out = []
    try:
        with open(path, "rb") as fh:
            for s in pe.sections[:MAX_SECTIONS]:
                if not s.SizeOfRawData:
                    continue
                fh.seek(s.PointerToRawData)
                data = fh.read(s.SizeOfRawData)
                if len(data) == s.SizeOfRawData:
                    out.append(digests([data]))
    finally:
        pe.close()
    return out


def record(tree, rel, outer=None):
    """Fingerprints of the programs and scripts under `tree` (and of the release file itself)."""
    found = []
    if outer:
        entry = {"file": os.path.basename(outer), **digests(chunks(outer))}
        found.append(entry)
    for root, _, files in os.walk(tree):
        for f in sorted(files):
            p = os.path.join(root, f)
            if len(found) >= MAX_FILES:
                return found, True
            if os.path.islink(p) or not os.path.isfile(p) or os.path.getsize(p) > MAX_HASHED or not program_like(p):
                continue
            entry = {"file": rel(p, tree), **digests(chunks(p))}
            with open(p, "rb") as fh:
                secs = sections(p) if fh.read(2) == b"MZ" else []
            if secs:
                entry["sections"] = secs
            found.append(entry)
    return found, False


# ---- matching ----------------------------------------------------------------------------------------

def signature_files(folder):
    """(name, lines) for every signature file, inside ClamAV's .cvd/.cld containers too."""
    for name in sorted(os.listdir(folder)):
        path = os.path.join(folder, name)
        if name.endswith((".cvd", ".cld")):
            with open(path, "rb") as fh:
                header = fh.read(512).decode("ascii", "replace")
                yield name + "#header", [header]
                try:
                    with tarfile.open(fileobj=fh, mode="r|*") as tar:
                        for member in tar:
                            if member.isfile() and member.name.endswith((".hdb", ".hsb", ".mdb", ".msb", ".fp", ".sfp", ".ign2")):
                                data = tar.extractfile(member)
                                yield member.name, (raw.decode("utf-8", "replace") for raw in data)
                except tarfile.TarError:
                    continue
        elif name.endswith((".hdb", ".hsb", ".mdb", ".msb", ".fp", ".sfp", ".ign2")):
            with open(path, encoding="utf-8", errors="replace") as fh:
                yield name, fh


def match(folder="/db/clamav", stdin=sys.stdin):
    files, secs = {}, {}
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        q = json.loads(line)
        index = secs if q.get("kind") == "section" else files
        for algo in ("md5", "sha1", "sha256"):
            if q.get(algo):
                index.setdefault(q[algo].lower(), []).append((q["id"], int(q["size"])))

    hits, allowed, ignored, versions = [], set(), set(), {}
    for name, lines in signature_files(folder):
        if name.endswith("#header"):
            parts = next(iter(lines)).split(":")
            if len(parts) > 2 and parts[0] == "ClamAV-VDB":
                versions[name.split(".")[0]] = parts[2]
            continue
        ext = name.rsplit(".", 1)[-1]
        for line in lines:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if ext == "ign2":
                ignored.add(line.split(":")[0])
                continue
            parts = line.split(":")
            if len(parts) < 3:
                continue
            if ext in ("mdb", "msb"):
                size, digest, sig = parts[0], parts[1].lower(), parts[2]
                index = secs
            else:
                digest, size, sig = parts[0].lower(), parts[1], parts[2]
                index = files
            if digest not in index:
                continue
            for qid, qsize in index[digest]:
                if size == "*" or (size.isdigit() and int(size) == qsize):
                    if ext in ("fp", "sfp"):
                        allowed.add((digest, qsize))
                    else:
                        hits.append((qid, sig, digest, qsize))
    matches, seen = [], set()
    for qid, sig, digest, size in hits:
        if sig in ignored or (digest, size) in allowed or (qid, sig) in seen:
            continue
        seen.add((qid, sig))
        matches.append({"id": qid, "name": sig})
    version = " ".join(f"{k} {v}" for k, v in sorted(versions.items()))
    print(json.dumps({"engines": {"clamav": version}, "matches": matches}))

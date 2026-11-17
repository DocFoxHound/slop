#!/usr/bin/env python3
"""Scans one release file and prints a verdict as JSON (crates/slop-api/src/scan.rs).

Static checks only; nothing in the file is ever run:
  - ClamAV: known malware, including inside archives;
  - YARA with YARA-Forge's rules (rules.py): malware families, hack tools, suspicious packers;
  - capa: what each program can do (keylogging, injecting into other processes, persistence…).

The file is opened up first, into a size-limited scratch folder (formats.py): archives (.zip,
.tar.*, .deb, .rpm, .7z…), and then what's inside them, found by their first bytes: installers
(NSIS, Inno Setup, MSI, self-extracting archives), Android and Java packages, Godot packs,
Electron's .asar, PyInstaller bundles, AppImages and UPX-packed programs.

Verdicts (DESIGN §10): "on_hold" when ClamAV or YARA's main rules find something (SLAPorSLOP
staff look at it: it may be a false positive), or when the file couldn't be scanned completely
(an archive that won't unpack, one past the size limits, an engine error or timeout, missing
signatures): a file that wasn't fully scanned never passes on its own. Something inside that we
open as an extra (an installer, a pack) and can't is a note, as before we opened them at all.
"note" when capa reports capabilities a reviewer should look at, or YARA's wider rules match,
else "passed". The detail is written for the maker and the reviewer.

The report also carries fingerprints of the programs and scripts inside (hashes.py), so later
ClamAV signatures can be checked against them without downloading the release again.

Usage: slop-scan <path> [name], or slop-scan - <name> with the file on stdin;
       slop-scan --match-hashes, with fingerprints on stdin (hashes.py).
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, "/usr/local/lib/slop-scan")
sys.path.insert(1, os.path.dirname(os.path.abspath(__file__)))

import formats  # noqa: E402
import hashes  # noqa: E402

MAX_UNPACKED = 4 * 1024**3
MAX_FILES = 20000
MAX_DEPTH = 4
# How long capa may take over all the programs in a file.
CAPA_BUDGET = 600
# Archives a release is expected to be, or carry: not opening one of these holds the file.
ARCHIVES = (".zip", ".tar", ".tgz", ".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2", ".deb", ".rpm", ".gz", ".xz")
# capa namespaces worth a reviewer's look. Networking is left out: most games talk to servers.
NOTABLE = (
    "anti-analysis/",
    "collection/keylog",
    "collection/screenshot",
    "collection/browser",
    "collection/credentials",
    "persistence/",
    "host-interaction/process/inject",
    "host-interaction/uac",
    "impact/",
    "data-manipulation/encryption/ransom",
)
UNPACKED = ".unpacked"


def run(args, timeout=600):
    return subprocess.run(args, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=timeout)


def size_of(folder):
    total, count = 0, 0
    for root, _, files in os.walk(folder):
        for f in files:
            p = os.path.join(root, f)
            if not os.path.islink(p):
                total += os.path.getsize(p)
                count += 1
    return total, count


def rel(path, root):
    if not path.startswith(root):
        return os.path.basename(path)
    return os.path.relpath(path, root).replace(UNPACKED, "")


class Unpacker:
    """Opens the release and everything inside it that we can, within the limits."""

    def __init__(self, tree):
        self.tree = tree
        self.total, self.count = 0, 0
        self.incomplete, self.notes = [], []

    def open(self, path, dest, required):
        name = rel(path, self.tree)
        try:
            result = formats.open_up(path, dest)
        except subprocess.TimeoutExpired:
            result = formats.Failed("took too long to open")
        if result is None:
            shutil.rmtree(dest, ignore_errors=True)
            if required:
                self.incomplete.append(f"couldn't unpack {name} (damaged, encrypted or too large)")
            return False
        if isinstance(result, formats.Failed):
            if required:
                self.incomplete.append(f"couldn't unpack {name} ({result.why})")
            else:
                self.notes.append(f"{name}: {result.why}, so its contents weren't scanned one by one")
            if not os.path.isdir(dest) or not os.listdir(dest):
                shutil.rmtree(dest, ignore_errors=True)
                return False
        total, count = size_of(dest)
        if self.total + total > MAX_UNPACKED or self.count + count > MAX_FILES:
            shutil.rmtree(dest, ignore_errors=True)
            if required:
                self.incomplete.append("too large to unpack fully")
            else:
                self.notes.append(f"{name} wasn't opened: past the size limits")
            return False
        self.total += total
        self.count += count
        return True

    def run(self, target):
        """Unpacks `target` into the tree (or copies it there) and then what's inside."""
        if target.lower().endswith(ARCHIVES):
            if not self.open(target, self.tree, required=True):
                os.makedirs(self.tree, exist_ok=True)
                return
        else:
            os.makedirs(self.tree, exist_ok=True)
            shutil.copy(target, os.path.join(self.tree, os.path.basename(target)))
        queue = [(self.tree, 1)]
        while queue and not self.incomplete:
            folder, depth = queue.pop(0)
            for root, dirs, files in os.walk(folder):
                dirs[:] = [d for d in dirs if not d.endswith(UNPACKED)]
                for f in sorted(files):
                    p = os.path.join(root, f)
                    if os.path.islink(p) or not os.path.isfile(p):
                        continue
                    required = f.lower().endswith(ARCHIVES)
                    if formats.kind_of(p) is None and not required:
                        continue
                    if depth >= MAX_DEPTH:
                        if required:
                            self.incomplete.append("archives nested too deep to unpack")
                            return
                        continue
                    dest = p + UNPACKED
                    if self.open(p, dest, required):
                        queue.append((dest, depth + 1))
                    elif self.incomplete:
                        return


def executables(folder, limit=8):
    """The biggest programs (Windows PE or Linux ELF) in the folder."""
    found = []
    for root, _, files in os.walk(folder):
        for f in files:
            p = os.path.join(root, f)
            if os.path.islink(p):
                continue
            try:
                with open(p, "rb") as fh:
                    magic = fh.read(4)
            except OSError:
                continue
            if magic[:2] == b"MZ" or magic == b"\x7fELF":
                found.append((os.path.getsize(p), p))
    return [p for _, p in sorted(found, reverse=True)[:limit]]


def yara(rules, tree):
    """(matches, problem) for one compiled rule set."""
    r = run(["yara", "-C", rules, "-r", "-w", "-f", tree])
    matches = []
    for line in r.stdout.splitlines():
        if " " in line:
            name, where = line.split(" ", 1)
            matches.append({"engine": "YARA", "file": rel(where, tree), "name": name})
    return matches, (r.returncode != 0)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--match-hashes":
        hashes.match()
        return
    work = tempfile.mkdtemp(prefix="scan-", dir=os.environ.get("TMPDIR", "/tmp"))
    target = sys.argv[1]
    if target == "-":
        # The file on stdin (slop-scan), saved under its name: only the extension matters.
        name = os.path.basename(sys.argv[2]) if len(sys.argv) > 2 else "file"
        os.makedirs(os.path.join(work, "in"))
        target = os.path.join(work, "in", name or "file")
        with open(target, "wb") as fh:
            shutil.copyfileobj(sys.stdin.buffer, fh, 1024 * 1024)
    findings, noticed, capabilities, notes, engines = [], [], [], [], {}
    fingerprints, capped = [], False
    # Why the file wasn't scanned completely; any of these holds it for SLAPorSLOP staff.
    incomplete = []
    try:
        tree = os.path.join(work, "files")
        unpacker = Unpacker(tree)
        unpacker.run(target)
        incomplete += unpacker.incomplete
        notes += unpacker.notes

        # ClamAV, on the file itself (it reads archives) and on what was unpacked.
        db = "/db/clamav"
        if os.path.isdir(db) and os.listdir(db):
            r = run(["clamscan", "--database", db, "-r", "--no-summary", "--infected", "--alert-encrypted=no",
                     "--max-filesize=1000M", "--max-scansize=4000M", target, tree])
            for line in r.stdout.splitlines():
                if line.endswith(" FOUND"):
                    where, name = line[: -len(" FOUND")].rsplit(": ", 1)
                    findings.append({"engine": "ClamAV", "file": rel(where, tree) if where.startswith(tree) else os.path.basename(where), "name": name})
            if r.returncode == 2:
                incomplete.append("ClamAV couldn't read everything")
            v = run(["clamscan", "--database", db, "--version"]).stdout.strip()
            engines["clamav"] = v
        else:
            incomplete.append("ClamAV has no signatures yet")

        # YARA: the main rules hold a file; the wider set's matches are notes (rules.py).
        hold = "/db/yara/hold.yarc"
        if not os.path.exists(hold):
            hold = "/db/yara/core.yarc"  # from before rules.py, until the next update
        if os.path.exists(hold):
            found, problem = yara(hold, tree)
            findings += found
            if problem:
                incomplete.append("YARA couldn't read everything")
            if os.path.exists("/db/yara/note.yarc"):
                found, _ = yara("/db/yara/note.yarc", tree)
                noticed += found
            engines["yara"] = "yara-forge " + open("/db/yara/version").read().strip()
        else:
            incomplete.append("YARA has no rules yet")

        # capa on the main programs, biggest first, within its time budget.
        deadline = time.monotonic() + CAPA_BUDGET
        for exe in executables(tree):
            left = deadline - time.monotonic()
            if left < 30:
                notes.append(f"capa ran out of time before {rel(exe, tree)}")
                break
            try:
                r = run(["capa", "-j", "-q", exe], timeout=min(300, left))
            except subprocess.TimeoutExpired:
                notes.append(f"capa took too long on {rel(exe, tree)}")
                continue
            if r.returncode != 0:
                continue
            try:
                report = json.loads(r.stdout)
            except json.JSONDecodeError:
                continue
            for name, rule in (report.get("rules") or {}).items():
                ns = (rule.get("meta") or {}).get("namespace") or ""
                if ns.startswith(NOTABLE):
                    capabilities.append({"file": rel(exe, tree), "capability": name, "namespace": ns})
        engines["capa"] = run(["capa", "--version"]).stdout.strip() or "capa"

        # The release file itself too, when it was unpacked rather than copied into the tree.
        outer = target if target.lower().endswith(ARCHIVES) else None
        fingerprints, capped = hashes.record(tree, rel, outer=outer)
    except subprocess.TimeoutExpired:
        incomplete.append("a scanner took too long")
    except OSError as e:
        incomplete.append(f"the scan stopped: {e.strerror or e}")
    finally:
        shutil.rmtree(work, ignore_errors=True)

    # A plain file is scanned as given and as copied into the tree: list each finding once.
    def once(items):
        unique = {}
        for f in items:
            unique.setdefault((f["engine"], f["file"], f["name"]), f)
        return list(unique.values())

    findings, noticed = once(findings), once(noticed)

    if findings:
        notes += incomplete
        result = "on_hold"
        names = sorted({f"{f['engine']}: {f['name']} in {f['file']}" for f in findings})
        detail = "Flagged by " + "; ".join(names[:5]) + (f" and {len(names) - 5} more" if len(names) > 5 else "") + \
            ". SLAPorSLOP staff look at it; it may be a false positive."
    elif incomplete:
        result = "on_hold"
        detail = "Not fully scanned: " + "; ".join(incomplete) + \
            ". SLAPorSLOP staff look at it."
    elif capabilities or noticed:
        result = "note"
        parts = []
        if capabilities:
            kinds = sorted({c["capability"] for c in capabilities})
            parts.append("it can: " + ", ".join(kinds[:8]) + (" and more" if len(kinds) > 8 else ""))
        if noticed:
            names = sorted({f"{f['name']} in {f['file']}" for f in noticed})
            parts.append("YARA's wider rules, which false-alarm more often, matched " + "; ".join(names[:3]) +
                         (f" and {len(names) - 3} more" if len(names) > 3 else ""))
        detail = "Scanned clean. For the reviewer, " + "; ".join(parts) + "."
    else:
        result = "passed"
        detail = "Scanned with ClamAV, YARA and capa: nothing found."
    if notes:
        detail += " (" + "; ".join(notes) + ")"
    print(json.dumps({"result": result, "detail": detail, "findings": findings, "noticed": noticed,
                      "capabilities": capabilities, "engines": engines,
                      "hashes": fingerprints, "hashes_capped": capped}))


if __name__ == "__main__":
    main()

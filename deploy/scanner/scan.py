#!/usr/bin/env python3
"""Scans one release file and prints a verdict as JSON (crates/slop-api/src/scan.rs).

Static checks only; nothing in the file is ever run:
  - ClamAV: known malware, including inside archives;
  - YARA with YARA-Forge's core rules: malware families, hack tools, suspicious packers;
  - capa: what each program can do (keylogging, injecting into other processes, persistence…).

Archives (.zip, .tar.*, .deb, .rpm) are unpacked into a size-limited scratch folder first.

Verdicts (DESIGN §10): "on_hold" when ClamAV or YARA finds something (a person checks it: it may
be a false positive), "note" when capa reports capabilities a reviewer should look at, else
"passed". The detail is written for the maker and the reviewer.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

MAX_UNPACKED = 4 * 1024**3
MAX_FILES = 20000
MAX_DEPTH = 3
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


def run(args, timeout=600):
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout)


def size_of(folder):
    total, count = 0, 0
    for root, _, files in os.walk(folder):
        for f in files:
            p = os.path.join(root, f)
            if not os.path.islink(p):
                total += os.path.getsize(p)
                count += 1
    return total, count


def unpack(path, dest, depth=0):
    """Unpacks archives recursively; returns a note if limits stopped it."""
    os.makedirs(dest, exist_ok=True)
    r = run(["bsdtar", "-x", "-f", path, "-C", dest, "--no-same-owner", "--no-same-permissions"], timeout=300)
    if r.returncode != 0:
        return None
    total, count = size_of(dest)
    if total > MAX_UNPACKED or count > MAX_FILES:
        return "too large to unpack fully"
    if depth >= MAX_DEPTH:
        return None
    for root, _, files in os.walk(dest):
        for f in files:
            p = os.path.join(root, f)
            if os.path.islink(p) or not f.lower().endswith(ARCHIVES):
                continue
            note = unpack(p, p + ".unpacked", depth + 1)
            if note:
                return note
    return None


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


def rel(path, root):
    return os.path.relpath(path, root).replace(".unpacked", "")


def main():
    target = sys.argv[1]
    work = tempfile.mkdtemp(prefix="scan-", dir=os.environ.get("TMPDIR", "/tmp"))
    findings, capabilities, notes, engines = [], [], [], {}
    try:
        tree = os.path.join(work, "files")
        if target.lower().endswith(ARCHIVES):
            note = unpack(target, tree)
            if note:
                notes.append(note)
        else:
            os.makedirs(tree)
            shutil.copy(target, os.path.join(tree, os.path.basename(target)))

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
                notes.append("ClamAV couldn't read everything")
            v = run(["clamscan", "--database", db, "--version"]).stdout.strip()
            engines["clamav"] = v
        else:
            notes.append("ClamAV has no signatures yet")

        # YARA-Forge core rules.
        rules = "/db/yara/core.yarc"
        if os.path.exists(rules):
            r = run(["yara", "-C", rules, "-r", "-w", "-f", tree])
            for line in r.stdout.splitlines():
                if " " in line:
                    name, where = line.split(" ", 1)
                    findings.append({"engine": "YARA", "file": rel(where, tree), "name": name})
            engines["yara"] = "yara-forge " + open("/db/yara/version").read().strip()
        else:
            notes.append("YARA has no rules yet")

        # capa on the main programs.
        for exe in executables(tree):
            try:
                r = run(["capa", "-j", "-q", exe], timeout=300)
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
    except subprocess.TimeoutExpired:
        notes.append("a scanner took too long")
    finally:
        shutil.rmtree(work, ignore_errors=True)

    if findings:
        result = "on_hold"
        names = sorted({f"{f['engine']}: {f['name']} in {f['file']}" for f in findings})
        detail = "Flagged by " + "; ".join(names[:5]) + (f" and {len(names) - 5} more" if len(names) > 5 else "") + \
            ". A person checks it before it can be approved; it may be a false positive."
    elif capabilities:
        result = "note"
        kinds = sorted({c["capability"] for c in capabilities})
        detail = "Scanned clean. For the reviewer, it can: " + ", ".join(kinds[:8]) + (" and more" if len(kinds) > 8 else "") + "."
    else:
        result = "passed"
        detail = "Scanned with ClamAV, YARA and capa: nothing found."
    if notes:
        detail += " (" + "; ".join(notes) + ")"
    print(json.dumps({"result": result, "detail": detail, "findings": findings,
                      "capabilities": capabilities, "engines": engines}))


if __name__ == "__main__":
    main()

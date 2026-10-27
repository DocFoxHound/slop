#!/usr/bin/env python3
"""Writes a CycloneDX 1.5 software bill of materials for one SLAPorSLOP build (SECURITY §13).

Usage: sbom.py <repo> <commit> > sbom.json

It lists the Rust crates actually built into the API and the desktop app (`cargo tree`, on any
platform; Cargo.lock also records optional dependencies nobody enables, which are left out) and
every package the website ships from web/package-lock.json (development-only packages left out),
with their versions, package URLs and checksums. Standard library only, so it runs in the deploy
script without installing anything. update.sh's dependency audit uses `built_crates` and
`web_packages` too, so it only reports what's really shipped.
"""

import json
import os
import subprocess
import sys
import tomllib
import uuid
from datetime import datetime, timezone


def built_crates(repo):
    """(name, version) of every crate compiled for some platform, build scripts included."""
    out = subprocess.run(
        ["cargo", "tree", "--workspace", "--target", "all", "-e", "normal,build", "--prefix", "none", "--format", "{p}"],
        cwd=repo, capture_output=True, text=True, check=True,
    ).stdout
    built = set()
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1].startswith("v"):
            built.add((parts[0], parts[1][1:]))
    return built


def web_packages(repo):
    """(name, version) of every package the website ships (no development-only ones)."""
    path = os.path.join(repo, "web", "package-lock.json")
    if not os.path.exists(path):
        return set()
    with open(path) as f:
        lock = json.load(f)
    return {
        (p.get("name") or key.split("node_modules/")[-1], p["version"])
        for key, p in lock.get("packages", {}).items()
        if key and not p.get("dev") and "version" in p
    }


def rust(repo):
    with open(os.path.join(repo, "Cargo.lock"), "rb") as f:
        lock = tomllib.load(f)
    try:
        built = built_crates(repo)
    except (OSError, subprocess.CalledProcessError):
        built = None
    for p in lock.get("package", []):
        if built is not None and (p["name"], p["version"]) not in built:
            continue
        c = {
            "type": "library",
            "name": p["name"],
            "version": p["version"],
            "purl": f"pkg:cargo/{p['name']}@{p['version']}",
            "bom-ref": f"cargo:{p['name']}@{p['version']}",
        }
        if p.get("checksum"):
            c["hashes"] = [{"alg": "SHA-256", "content": p["checksum"]}]
        if "source" not in p:
            # One of SLAPorSLOP's own crates.
            c["type"] = "application"
            del c["purl"]
        yield c


def web(repo):
    path = os.path.join(repo, "web", "package-lock.json")
    if not os.path.exists(path):
        return
    with open(path) as f:
        lock = json.load(f)
    for key, p in lock.get("packages", {}).items():
        if not key or p.get("dev") or "version" not in p:
            continue
        name = p.get("name") or key.split("node_modules/")[-1]
        purl_name = name.replace("@", "%40", 1) if name.startswith("@") else name
        c = {
            "type": "library",
            "name": name,
            "version": p["version"],
            "purl": f"pkg:npm/{purl_name}@{p['version']}",
            "bom-ref": f"npm:{name}@{p['version']}",
        }
        integrity = p.get("integrity", "")
        if integrity.startswith("sha512-"):
            import base64
            c["hashes"] = [{"alg": "SHA-512", "content": base64.b64decode(integrity[7:]).hex()}]
        yield c


def main():
    repo, commit = sys.argv[1], sys.argv[2]
    components = list(rust(repo)) + list(web(repo))
    seen = set()
    unique = []
    for c in components:
        if c["bom-ref"] not in seen:
            seen.add(c["bom-ref"])
            unique.append(c)
    bom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": f"urn:uuid:{uuid.uuid4()}",
        "version": 1,
        "metadata": {
            "timestamp": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "component": {
                "type": "application",
                "name": "slop",
                "version": commit,
                "bom-ref": "slop",
            },
        },
        "components": unique,
    }
    json.dump(bom, sys.stdout, indent=1)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()

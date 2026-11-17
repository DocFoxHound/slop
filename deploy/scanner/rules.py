#!/usr/bin/env python3
"""Builds the scanner's two YARA rule sets from YARA-Forge's packages (update.sh runs this).

Usage: slop-scan-rules <core.yar> <extended.yar> <out folder>

- hold.yar: the core package (the curated, low false-positive rules), plus the extended package's
  rules YARA-Forge rates as good (quality 70 or more) and likely malicious (score 75 or more):
  mostly older rules, left out of core by age. A match holds the file for staff.
- note.yar: the rest of the extended package, which false-alarms more. A match is a note.

Only rules whose licence allows SLAPorSLOP to use them are kept: a rule needs a licence (its
repository's, or its own `license` field) and none that forbids commercial use. Rules a kept rule
depends on (private rules) come along; a rule depending on one that was dropped is dropped too.
"""

import os
import re
import sys

HOLD_SCORE = 75
HOLD_QUALITY = 70
NOT_COMMERCIAL = re.compile(r"non-?commercial|\bNC\b|BY-NC", re.I)
IDENT = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")
META = re.compile(r'\b(\w+)\s*=\s*(?:"((?:[^"\\]|\\.)*)"|(-?\d+|true|false)\b)')


def finish(block, rules):
    text = "".join(block)
    head = re.match(r"((?:private |global )*)rule\s+([A-Za-z_][A-Za-z0-9_]*)", text)
    section = text.split("meta:", 1)[1] if "meta:" in text else ""
    section = re.split(r"\b(?:strings|condition):", section, maxsplit=1)[0]
    meta = {k: q if q or not n else n for k, q, n in META.findall(section)}
    rules[head.group(2)] = (text, meta, "private" in head.group(1))


def parse(path):
    """The file's imports and its rules, as {name: (text, meta, private)} in file order."""
    imports, rules, block = [], {}, None
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            if block is None:
                if line.startswith("import "):
                    imports.append(line.strip())
                elif re.match(r"(private |global )*rule ", line):
                    block = [line]
                    # A rule written on one line ends where it starts.
                    if "condition:" in line and line.rstrip().endswith("}"):
                        finish(block, rules)
                        block = None
                continue
            block.append(line)
            if line.rstrip() == "}" and any("condition:" in b for b in block):
                finish(block, rules)
                block = None
    return imports, rules


def licensed(meta):
    named = [v for k, v in meta.items() if k in ("license", "malpedia_license")]
    url = meta.get("license_url", "N/A")
    if not named and url in ("", "N/A"):
        return False
    return not any(NOT_COMMERCIAL.search(v) for v in named)


def number(meta, key):
    try:
        return int(meta.get(key, "0"))
    except ValueError:
        return 0


def needs(text, names, own):
    condition = text.split("condition:", 1)[-1]
    return {t for t in IDENT.findall(condition) if t in names and t != own}


def main():
    core_path, extended_path, out = sys.argv[1:4]
    imports, extended = parse(extended_path)
    core_imports, core = parse(core_path)
    imports = sorted(set(imports) | set(core_imports))
    rules = {**extended, **core}
    names = set(rules)
    deps = {n: needs(t, names, n) for n, (t, _, _) in rules.items()}

    usable = {n for n, (_, m, _) in rules.items() if licensed(m)}
    changed = True
    while changed:
        changed = False
        for n in list(usable):
            if not deps[n] <= usable:
                usable.discard(n)
                changed = True

    public = [n for n in rules if n in usable and not rules[n][2]]
    hold = [n for n in public if n in core
            or (number(rules[n][1], "score") >= HOLD_SCORE and number(rules[n][1], "quality") >= HOLD_QUALITY)]
    held = set(hold)
    note = [n for n in public if n not in held]

    def closure(chosen):
        keep, todo = set(chosen), list(chosen)
        while todo:
            for d in deps[todo.pop()]:
                if d not in keep:
                    keep.add(d)
                    todo.append(d)
        return [n for n in rules if n in keep]

    for name, chosen in (("hold", hold), ("note", note)):
        with open(os.path.join(out, name + ".yar"), "w", encoding="utf-8") as fh:
            fh.write("\n".join(imports) + "\n\n")
            for n in closure(chosen):
                fh.write(rules[n][0] + "\n")
    dropped = len([n for n in rules if not rules[n][2]]) - len(public)
    print(f"YARA rules: {len(hold)} hold, {len(note)} note, {dropped} left out (licence)")


if __name__ == "__main__":
    main()

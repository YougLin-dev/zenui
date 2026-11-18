#!/usr/bin/env python3
"""Validate the docs/ corpus against the governance contract in docs/README.md.

Checks, in order:
  1. every document outside docs/archive has valid frontmatter;
  2. every relative Markdown link resolves to an existing file and anchor;
  3. every SPEC/CLAIM/ADR/EVID reference points at a defined ID;
  4. every VERIFIED/PROBE/WITHDRAWN claim carries an evidence reference.

Exit code is the number of failures, capped at 99. Standard library only.
"""

from __future__ import annotations

import os
import re
import sys
import unicodedata

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOCS = os.path.join(ROOT, "docs")
RECORDS = os.path.join(DOCS, "evidence")

REQUIRED_KEYS = ("doc-id", "layer", "status", "date", "owner", "sources", "replaces")
LAYERS = {"spec", "evidence", "decisions", "archive", "product", "governance"}
STATUSES = {"draft", "normative", "frozen", "historical"}
ARCHIVE_PREFIXES = ("docs/archive/research/", "docs/archive/review/")

ID_REF = re.compile(
    r"\b(?:SPEC-[A-Z0-9]+(?:-[A-Z0-9]+)*-\d{2,3}"
    r"|EVID-(?:VAL|PROBE)-\d{2}"
    r"|EVID-MATRIX"
    r"|CLAIM-\d{3}"
    r"|ADR-\d{4})\b"
)
HEADING_DEF = re.compile(r"^#{2,6}\s+`?([A-Z][A-Z0-9-]*-[A-Z0-9]+-\d{2,3}|CLAIM-\d{3})\b")
TABLE_DEF = re.compile(r"^\|\s*`?([A-Z][A-Z0-9-]*-[A-Z0-9]+-\d{2,3}|CLAIM-\d{3})`?\s*\|")
CLAIM_ROW = re.compile(r"^\|\s*(CLAIM-\d{3})\s*\|")
EVIDENCE_REQUIRED = {"VERIFIED", "PROBE", "WITHDRAWN"}

failures: list[str] = []


def rel(path: str) -> str:
    return os.path.relpath(path, ROOT)


def markdown_files() -> list[str]:
    out = []
    for dirpath, _dirnames, filenames in os.walk(DOCS):
        for name in filenames:
            if name.endswith(".md"):
                out.append(os.path.join(dirpath, name))
    return sorted(out)


def is_archive(path: str) -> bool:
    return rel(path).replace(os.sep, "/").startswith(ARCHIVE_PREFIXES)


def parse_frontmatter(text: str) -> dict[str, str] | None:
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return None
    fields: dict[str, str] = {}
    for line in lines[1:]:
        if line.strip() == "---":
            return fields
        if not line.strip() or line.lstrip().startswith("- "):
            continue
        key, sep, value = line.partition(":")
        if sep:
            fields[key.strip()] = value.strip()
    return None


def check_frontmatter(files: list[str]) -> None:
    for path in files:
        if is_archive(path):
            continue  # ADR-0013: archived originals stay byte-identical
        fields = parse_frontmatter(open(path, encoding="utf-8").read())
        if fields is None:
            failures.append(f"{rel(path)}: missing frontmatter")
            continue
        for key in REQUIRED_KEYS:
            if key not in fields:
                failures.append(f"{rel(path)}: frontmatter missing key '{key}'")
        if fields.get("layer") and fields["layer"] not in LAYERS:
            failures.append(f"{rel(path)}: unknown layer '{fields['layer']}'")
        if fields.get("status") and fields["status"] not in STATUSES:
            failures.append(f"{rel(path)}: unknown status '{fields['status']}'")
        is_template = rel(path).replace(os.sep, "/").startswith("docs/templates/")
        if not is_template and fields.get("date") and not re.fullmatch(
            r"\d{4}-\d{2}-\d{2}", fields["date"]
        ):
            failures.append(f"{rel(path)}: malformed date '{fields['date']}'")


def slug(text: str) -> str:
    text = re.sub(r"`([^`]*)`", r"\1", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = re.sub(r"[*_~]", "", text).strip().lower()
    kept = []
    for ch in text:
        if ch.isalnum() or ch in " -_":
            kept.append(ch)
        elif unicodedata.category(ch).startswith("C"):
            continue
    return re.sub(r"\s", "-", "".join(kept))


def anchors(text: str) -> set[str]:
    seen: dict[str, int] = {}
    out: set[str] = set()
    for line in text.splitlines():
        m = re.match(r"^#{1,6}\s+(.*?)\s*#*\s*$", line)
        if not m:
            continue
        base = slug(m.group(1))
        count = seen.get(base, 0)
        out.add(base if count == 0 else f"{base}-{count}")
        seen[base] = count + 1
    return out


def check_links(files: list[str]) -> None:
    cache: dict[str, set[str]] = {}
    for path in files:
        text = open(path, encoding="utf-8").read()
        for match in re.finditer(r"\]\(([^)\s]+)\)", text):
            target = match.group(1)
            if target.startswith(("http://", "https://", "mailto:")):
                continue
            file_part, _, anchor = target.partition("#")
            if not file_part:
                if anchor and anchor not in anchors(text):
                    failures.append(f"{rel(path)}: dead anchor '#{anchor}'")
                continue
            resolved = os.path.normpath(os.path.join(os.path.dirname(path), file_part))
            if not os.path.exists(resolved):
                failures.append(f"{rel(path)}: dead link '{target}'")
                continue
            if anchor and resolved.endswith(".md"):
                if resolved not in cache:
                    cache[resolved] = anchors(open(resolved, encoding="utf-8").read())
                if anchor not in cache[resolved]:
                    failures.append(f"{rel(path)}: dead anchor '{target}'")


def defined_ids(files: list[str]) -> set[str]:
    ids: set[str] = set()
    for path in files:
        if is_archive(path):
            continue
        text = open(path, encoding="utf-8").read()
        fields = parse_frontmatter(text) or {}
        doc_id = fields.get("doc-id")
        if doc_id:
            ids.add(doc_id)
        for line in text.splitlines():
            for pattern in (HEADING_DEF, TABLE_DEF):
                m = pattern.match(line)
                if m:
                    ids.add(m.group(1))
        m = re.search(r"^adr-(\d{4})-", os.path.basename(path))
        if m:
            ids.add(f"ADR-{m.group(1)}")
    for name in os.listdir(RECORDS):
        if not name.endswith(".json"):
            continue
        head = open(os.path.join(RECORDS, name), encoding="utf-8").read(512)
        m = re.search(r'"docId"\s*:\s*"([A-Z][A-Z0-9-]+)"', head)
        if m:
            ids.add(m.group(1))
    return ids


def check_ids(files: list[str]) -> None:
    known = defined_ids(files)
    # References to IDs above the highest defined one are external (for example
    # FLUI's ADR-0304) or reserved for the next entry (EVID-VAL-20 in the
    # "how to add an entry" section), so they carry no obligation to resolve.
    max_adr = max((int(i[4:]) for i in known if re.fullmatch(r"ADR-\d{4}", i)), default=0)
    max_val = max((int(i.rsplit("-", 1)[1]) for i in known if i.startswith("EVID-VAL-")), default=0)
    for path in files:
        if is_archive(path):
            continue
        text = open(path, encoding="utf-8").read()
        for line_no, line in enumerate(text.splitlines(), 1):
            for ref in ID_REF.findall(line):
                if ref in known:
                    continue
                if ref in line and (HEADING_DEF.match(line) or TABLE_DEF.match(line)):
                    continue  # the definition itself, or a bare list row
                if ref.startswith("ADR-") and int(ref[4:]) > max_adr:
                    continue  # external decision record, e.g. FLUI's ADR-0304
                if ref.startswith("EVID-VAL-") and int(ref.rsplit("-", 1)[1]) == max_val + 1:
                    continue  # reserved next id, not yet an entry
                failures.append(f"{rel(path)}:{line_no}: unresolved ID '{ref}'")


def check_claim_evidence(files: list[str]) -> None:
    for path in files:
        if is_archive(path) or not path.endswith("claim-register.md"):
            continue
        for line_no, line in enumerate(open(path, encoding="utf-8"), 1):
            m = CLAIM_ROW.match(line)
            if not m:
                continue
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if len(cells) < 5:
                failures.append(f"{rel(path)}:{line_no}: malformed claim row")
                continue
            status, evidence = cells[2], cells[3]
            if status in EVIDENCE_REQUIRED and evidence in ("", "—", "-"):
                failures.append(
                    f"{rel(path)}:{line_no}: {m.group(1)} is {status} without evidence"
                )


def main() -> int:
    files = markdown_files()
    check_frontmatter(files)
    check_links(files)
    check_ids(files)
    check_claim_evidence(files)
    if failures:
        for item in failures:
            print(item)
        print(f"\n{len(failures)} problem(s) in {len(files)} Markdown files")
        return min(len(failures), 99)
    print(f"docs check passed: {len(files)} Markdown files")
    return 0


if __name__ == "__main__":
    sys.exit(main())

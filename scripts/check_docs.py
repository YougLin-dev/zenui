#!/usr/bin/env python3
"""Validate the docs/ corpus against the governance contract in docs/README.md.

Checks, in order:
  1. every document outside docs/archive has valid frontmatter;
  2. every relative Markdown link resolves to an existing file and anchor;
  3. every SPEC/CLAIM/ADR/EVID reference points at a defined ID;
  4. every ID is defined once (a ledger's own index table may repeat the id of
     the entry it points at, nothing else may);
  5. every VERIFIED/PROBE/WITHDRAWN claim carries an evidence reference;
  6. docs/evidence/matrix.json is internally consistent: run ids are
     contiguous, counts/coverage are recomputable from runs[] and gates[],
     and every gate id it projects is defined in the spec;
  7. the derived numbers quoted by the entry documents are recomputed from the
     corpus rather than trusted: the EVID-VAL range, the coverage split, the
     runsTotal/testsPassedH2/runsReproducibleHere triple and the CLAIM totals;
  8. matrix.json runs[] and the validation.md index describe the same runs,
     and the claim-register evidence-key table covers every EVID-VAL;
  9. every commit a run cites exists in this repository and is an ancestor of
     HEAD, because a rewritten commit leaves the id and the numbers behind;
 10. matrix.json criteria[] carries the ADR-0024 close predicate: every
     criterion's obligations concatenate back to the exact text of its spec
     column, every ref resolves to a #[test] in the tree or to a command that
     appears verbatim in validation.md, every notApplicable obligation names a
     defined ADR and an acceptedBy target whose text has landed, and both
     criteria[].closed and gates[].closed equal the recomputation. Nothing is
     read from a hand-maintained "closed" summary: hand-written booleans drift.

This file only reads. Generation of criteria[] (or any other matrix data) is a
separate, deliberate step; CI must never rewrite the corpus.

Exit code is the number of failures, capped at 99. Standard library only.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import unicodedata
from collections import Counter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOCS = os.path.join(ROOT, "docs")
RECORDS = os.path.join(DOCS, "evidence")
MATRIX = os.path.join(RECORDS, "matrix.json")
VALIDATION = os.path.join(RECORDS, "validation.md")
CLAIMS = os.path.join(RECORDS, "claim-register.md")
GATES = os.path.join(DOCS, "spec", "08-gates.md")

# Documents that quote corpus-wide derived numbers and must stay in sync with
# evidence/matrix.json. spec/ is frozen and decisions/ ADRs are immutable, so
# neither may be used as a ledger of current derived counts.
ENTRY_DOCUMENTS = ("docs/README.md", "docs/evidence/index.md")

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
STATUS_WORDS = ("VERIFIED", "PROBE", "OPEN", "SPEC", "WITHDRAWN")

VALIDATION_ROW = re.compile(r"^\|\s*(EVID-VAL-\d{2})\s*\|")
CLAIM_SUMMARY_ROW = re.compile(
    r"^\|\s*(VERIFIED|PROBE|OPEN|SPEC|WITHDRAWN)\s*\|\s*(\d+)\s*\|"
)
CLAIM_TOTAL_ROW = re.compile(
    r"^\|\s*\*{0,2}(?:合计|总计)\*{0,2}\s*\|\s*\*{0,2}(\d+)\*{0,2}\s*\|"
)
EVID_RANGE = re.compile(r"EVID-VAL-01\s*(?:…|\.{2,3})\s*(\d+)")
# One labelled count per status word, matched in any order so a reworded
# sentence is caught instead of silently skipped.
COVERAGE_MARK = re.compile(r"(\d+)\s*条(?:判据|Gate 判据)")
COVERAGE_LABEL = re.compile(r"(\d+)\s*条?\s*[`*]*\s*(VERIFIED|PROBE|OPEN)\b")
# ADR-0024 decision 4: "12 PROBE (of which N closed)" is the entry-document
# form. `closed` is not a status word; it is parsed separately.
COVERAGE_CLOSED = re.compile(r"其中\s*(\d+)\s*条?\s*[`*]*closed[`*]*")
# A test is a `#[test]`-annotated `fn`; the equivalent of `cargo test -- --list`
# that works without a Rust toolchain (the docs job installs Python only).
TEST_ATTR = re.compile(r"^\s*#\[test\]\s*$")
TEST_FN = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)")
OBLIGATION_SOURCES = ("criterion", "evidence")
# A recorded command is a `$ ...` line in validation.md, not any prose substring.
RECORDED_CMD = re.compile(r"^\s*\$\s+(.+?)\s*$")
GATE_ROW = re.compile(r"^\|\s*`?(SPEC-GATE-[A-Z0-9]+-\d{2})`?\s*\|")
GATE_STATUS = re.compile(r"^`(VERIFIED|PROBE|OPEN|SPEC|WITHDRAWN)`")
DERIVED_KEYS = ("runsTotal", "testsPassedH2", "runsReproducibleHere", "runsTranscribedOrExternal")
KEY_VALUE = re.compile(r"\b(" + "|".join(DERIVED_KEYS) + r")\s*=\s*(\d+)")

RUN_REQUIRED = ("id", "class", "host", "kind", "command")
RUN_CLASSES = {"A", "B", "C", "D"}
COMMIT_SHA = re.compile(r"[0-9a-f]{40}")

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


def check_ids(files: list[str], known: set[str]) -> None:
    # References to an ADR above the highest defined one are external records
    # (for example FLUI's ADR-0304) and carry no obligation to resolve here.
    # Nothing else is exempt: an ID defined on the same line is already in
    # `known`, so a row that cites EVID-VAL-99 is dangling whatever else it is.
    max_adr = max((int(i[4:]) for i in known if re.fullmatch(r"ADR-\d{4}", i)), default=0)
    for path in files:
        if is_archive(path):
            continue
        text = read_text(path)
        for line_no, line in enumerate(text.splitlines(), 1):
            for ref in ID_REF.findall(line):
                if ref in known:
                    continue
                if ref.startswith("ADR-") and int(ref[4:]) > max_adr:
                    continue  # external decision record, e.g. FLUI's ADR-0304
                failures.append(f"{rel(path)}:{line_no}: unresolved ID '{ref}'")


def check_claim_evidence(files: list[str]) -> None:
    for path in files:
        if is_archive(path) or not path.endswith("claim-register.md"):
            continue
        for line_no, line in enumerate(open(path, encoding="utf-8"), 1):
            m = CLAIM_ROW.match(line)
            if not m:
                continue
            cells = split_row(line)
            if len(cells) < 5:
                failures.append(f"{rel(path)}:{line_no}: malformed claim row")
                continue
            status, evidence = cells[2], cells[3]
            if status in EVIDENCE_REQUIRED and evidence in ("", "—", "-"):
                failures.append(
                    f"{rel(path)}:{line_no}: {m.group(1)} is {status} without evidence"
                )


def read_text(path: str) -> str:
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def split_row(line: str) -> list[str]:
    """Cells of a Markdown table row; an escaped `\\|` stays inside its cell."""
    return [cell.strip() for cell in re.split(r"(?<!\\)\|", line.strip().strip("|"))]


def check_duplicate_ids(files: list[str]) -> None:
    """Every ID is defined once.

    A ledger file repeats the id of each entry in its own index table and then
    opens a section for it; that pair is the only sanctioned duplication and is
    capped at one table row plus one heading per file.
    """
    definitions: dict[str, list[tuple[str, str]]] = {}
    for path in files:
        if is_archive(path):
            continue
        text = read_text(path)
        fields = parse_frontmatter(text) or {}
        local: dict[str, list[str]] = {}
        if fields.get("doc-id"):
            local.setdefault(fields["doc-id"], []).append("frontmatter")
        for line in text.splitlines():
            for kind, pattern in (("heading", HEADING_DEF), ("table", TABLE_DEF)):
                m = pattern.match(line)
                if m:
                    local.setdefault(m.group(1), []).append(kind)
        m = re.search(r"^adr-(\d{4})-", os.path.basename(path))
        if m:
            local.setdefault(f"ADR-{m.group(1)}", []).append("filename")
        # A document declaring its own doc-id (frontmatter, or the ADR file
        # name) is one definition, not two.
        for kinds in local.values():
            if {"frontmatter", "filename"} <= set(kinds):
                kinds.remove("filename")
        for doc_id, kinds in local.items():
            definitions.setdefault(doc_id, []).extend((rel(path), kind) for kind in kinds)

    for doc_id, places in sorted(definitions.items()):
        if doc_id.startswith(("EVID-VAL-", "EVID-PROBE-")):
            # One entry section in the whole corpus, one index row per table.
            headings = [path for path, kind in places if kind == "heading"]
            if len(headings) > 1:
                failures.append(
                    f"{doc_id} has {len(headings)} entry headings "
                    f"({', '.join(headings)}); exactly one is allowed"
                )
            per_file: dict[str, int] = {}
            for path, kind in places:
                if kind == "table":
                    per_file[path] = per_file.get(path, 0) + 1
            for path, count in sorted(per_file.items()):
                if count > 1:
                    failures.append(f"{path}: {doc_id} appears in {count} index rows; one is allowed")
        elif len(places) > 1:
            where = ", ".join(f"{path} ({kind})" for path, kind in places)
            failures.append(f"duplicate ID '{doc_id}' defined at {where}")


TEMPLATE_RESIDUE = re.compile(r"%[A-Z][A-Z0-9_]*(?:_[A-Z0-9]+)*%")
# `%TEMP%` and friends are real environment variables, not template tokens.
TEMPLATE_ALLOWED = {"%TEMP%", "%APPDATA%", "%USERPROFILE%", "%LOCALAPPDATA%"}


def check_template_residue(files: list[str]) -> None:
    """A literal `%COMMIT%` in a ledger is a missing fact, not a placeholder to
    be filled later: no script renders one, so it reads as a commit that never
    existed. Fail on it like any other bad record."""
    for path in files:
        if is_archive(path):
            continue
        for line_no, line in enumerate(read_text(path).splitlines(), 1):
            for match in TEMPLATE_RESIDUE.finditer(line):
                if match.group(0) in TEMPLATE_ALLOWED:
                    continue
                failures.append(
                    f"{rel(path)}:{line_no}: unresolved template placeholder "
                    f"{match.group(0)!r}"
                )


def load_matrix() -> dict | None:
    try:
        return json.loads(read_text(MATRIX))
    except (OSError, json.JSONDecodeError) as exc:
        failures.append(f"{rel(MATRIX)}: unreadable ({exc})")
        return None


def check_matrix_runs(matrix: dict) -> None:
    runs = matrix.get("runs")
    if not isinstance(runs, list) or not runs:
        failures.append(f"{rel(MATRIX)}: 'runs' must be a non-empty list")
        return
    host_ids = {host.get("id") for host in matrix.get("hosts") or [] if isinstance(host, dict)}
    seen: set[str] = set()
    for index, run in enumerate(runs):
        where = f"{rel(MATRIX)} runs[{index}]"
        if not isinstance(run, dict):
            failures.append(f"{where}: must be an object")
            continue
        for key in RUN_REQUIRED:
            if not run.get(key):
                failures.append(f"{where}: missing '{key}'")
        run_id = run.get("id", "")
        if run_id in seen:
            failures.append(f"{where}: duplicate run id '{run_id}'")
        seen.add(run_id)
        if run.get("class") not in RUN_CLASSES:
            failures.append(f"{where}: unknown class '{run.get('class')}'")
        if host_ids and run.get("host") not in host_ids:
            failures.append(f"{where}: host '{run.get('host')}' not declared in 'hosts'")
        tests = run.get("tests")
        if isinstance(tests, dict) and "platforms" in tests and tests["platforms"] != platform_count(run):
            failures.append(
                f"{where}: tests.platforms={tests['platforms']!r} but the run lists "
                f"{platform_count(run)} platform(s)"
            )

    expected = [f"EVID-VAL-{n:02d}" for n in range(1, len(runs) + 1)]
    actual = [run.get("id") if isinstance(run, dict) else None for run in runs]
    if actual != expected:
        failures.append(
            f"{rel(MATRIX)}: runs[] must be EVID-VAL-01…{len(runs):02d} in order, "
            f"found {actual}"
        )


def passed_tests(run: dict, default: int = 0) -> int:
    tests = run.get("tests")
    if isinstance(tests, dict) and isinstance(tests.get("passed"), int):
        return tests["passed"]
    return default


def platform_count(run: dict) -> int:
    """A run may hold one command executed on several CI runners at once."""
    platforms = run.get("platforms")
    return len(platforms) if isinstance(platforms, list) and platforms else 1


def matrix_counts(matrix: dict) -> dict[str, int]:
    """Recompute the derived counters from runs[]. Validation.md §9 fixes the
    definitions: class A runs are reproducible here, B/C are transcribed or
    external, and H2 is the host that produced this round's numbers."""
    runs = [run for run in matrix.get("runs") or [] if isinstance(run, dict)]
    derived = {
        "runsTotal": len(runs),
        "testsPassedH2": sum(passed_tests(run) for run in runs if run.get("host") == "H2"),
        "compileFailFixturesH2": sum(
            run.get("compileFailFixtures", 0)
            for run in runs
            if run.get("host") == "H2" and isinstance(run.get("compileFailFixtures", 0), int)
        ),
        "gpuBackendsObserved": len(
            {run.get("adapter") for run in runs if run.get("kind") == "gpu-probe" and run.get("adapter")}
        ),
        "runsReproducibleHere": sum(1 for run in runs if run.get("class") == "A"),
        "runsTranscribedOrExternal": sum(1 for run in runs if run.get("class") in {"B", "C"}),
        "runsOnCi": sum(1 for run in runs if run.get("host") == "CI"),
        # Execution count, not distinct tests: 40 tests on 3 runners is 120
        # executions, and it must never be folded into testsPassedH2.
        "testsPassedCi": sum(
            passed_tests(run) * platform_count(run) for run in runs if run.get("host") == "CI"
        ),
    }
    derived["zeroTestTargetsH2"] = sum(
        1
        for run in runs
        if run.get("host") == "H2"
        and run.get("kind") == "test"
        and isinstance(run.get("tests"), dict)
        and passed_tests(run, 1) == 0
    )
    return derived


def check_matrix_counts(matrix: dict) -> None:
    counts = matrix.get("counts") or {}
    for key, value in matrix_counts(matrix).items():
        stored = counts.get(key)
        if isinstance(stored, list):
            if len(stored) != value:
                failures.append(
                    f"{rel(MATRIX)} counts.{key} lists {len(stored)} entries "
                    f"but runs[] imply {value}"
                )
        elif stored != value:
            failures.append(
                f"{rel(MATRIX)} counts.{key}={stored!r} but runs[] recompute to {value!r}"
            )

    # Every run belongs to exactly one provenance class, so the three buckets
    # must add up to runsTotal; otherwise a new class silently escapes a count.
    derived = matrix_counts(matrix)
    partition = (
        derived["runsReproducibleHere"] + derived["runsTranscribedOrExternal"] + derived["runsOnCi"]
    )
    if partition != derived["runsTotal"]:
        failures.append(
            f"{rel(MATRIX)}: A + (B/C) + CI cover {partition} runs but runsTotal is "
            f"{derived['runsTotal']}"
        )


def spec_rows() -> dict[str, dict[str, str]]:
    """Criterion id -> gate, both text columns and status, from the gate table."""
    rows: dict[str, dict[str, str]] = {}
    for line in read_text(GATES).splitlines():
        m = GATE_ROW.match(line)
        if not m:
            continue
        cells = split_row(line)
        status = GATE_STATUS.match(cells[-1]) if cells else None
        if not status:
            failures.append(f"{rel(GATES)}: {m.group(1)} has no status word in its last cell")
            continue
        rows[m.group(1)] = {
            "gate": m.group(1).rsplit("-", 1)[0],
            "must": cells[1],
            "evidence": cells[2],
            "status": status.group(1),
        }
    return rows


def strip_text(text: str) -> str:
    """ADR-0024: whitespace and the full-width semicolon are separators, not text.

    Backticks and emphasis are kept: they are part of the obligation.
    """
    return re.sub(r"[\s；]", "", text)


def strip_rust_noise(text: str) -> str:
    """Blank comments and string/char literals before looking for attributes.

    A `#[test]` inside a block comment or a string is not a test; without this
    the manifest would accept a name that no test defines. The scan is a single
    left-to-right pass because treating comments and literals in separate regex
    passes lets a `"/*"` in a string swallow the rest of the file.
    """
    out: list[str] = []
    index = 0
    length = len(text)
    while index < length:
        char = text[index]
        if text.startswith("//", index):
            newline = text.find("\n", index)
            index = length if newline < 0 else newline
            continue
        if text.startswith("/*", index):
            close = text.find("*/", index + 2)
            index = length if close < 0 else close + 2
            continue
        if char == "r" and not (index and (text[index - 1].isalnum() or text[index - 1] == "_")):
            cursor, hashes = index + 1, 0
            while cursor < length and text[cursor] == "#":
                hashes += 1
                cursor += 1
            if cursor < length and text[cursor] == '"':
                terminator = '"' + "#" * hashes
                close = text.find(terminator, cursor + 1)
                index = length if close < 0 else close + len(terminator)
                continue
        if char == '"':
            cursor = index + 1
            while cursor < length:
                if text[cursor] == "\\":
                    cursor += 2
                    continue
                if text[cursor] == '"':
                    cursor += 1
                    break
                cursor += 1
            index = cursor
            continue
        if char == "'":
            literal = re.match(r"'(?:\\.|[^'\\])'", text[index:])
            if literal:
                index += literal.end()
                continue
        out.append(char)
        index += 1
    return "".join(out)


def test_manifest() -> set[str]:
    """Every `#[test]` function name under crates/ and examples/.

    This is the equivalent source listing ADR-0024 allows in place of
    `cargo test -- --list`: the docs job installs Python but no Rust toolchain,
    so the predicate must be computable without building the workspace.
    """
    names: set[str] = set()
    # Only the cargo workspace (`crates/`): a `#[test]` in an examples/* PoC is
    # not in `cargo test --workspace -- --list` and must not back a gate clause.
    for base in ("crates",):
        for dirpath, _dirnames, filenames in os.walk(os.path.join(ROOT, base)):
            for name in filenames:
                if not name.endswith(".rs"):
                    continue
                lines = strip_rust_noise(read_text(os.path.join(dirpath, name))).splitlines()
                for index, line in enumerate(lines):
                    if not TEST_ATTR.match(line):
                        continue
                    for following in lines[index + 1 : index + 4]:
                        m = TEST_FN.match(following)
                        if m:
                            names.add(m.group(1))
                            break
    return names


def recorded_commands(text: str) -> set[str]:
    """Commands actually recorded in validation.md, as `$ <command>` lines."""
    commands = set()
    for match in (RECORDED_CMD.match(line) for line in text.splitlines()):
        if not match:
            continue
        command = re.sub(r"\s+#.*$", "", match.group(1)).strip()
        if command and not command.startswith("#"):
            commands.add(command)
    return commands


def adr_documents() -> dict[str, tuple[str, str]]:
    """ADR id -> (frontmatter status, body text).

    A transfer may only cite a normative ADR whose body actually names the
    criterion it transfers; otherwise any ADR would do.
    """
    documents: dict[str, tuple[str, str]] = {}
    directory = os.path.join(DOCS, "decisions")
    for name in os.listdir(directory):
        if not (name.startswith("adr-") and name.endswith(".md")):
            continue
        text = read_text(os.path.join(directory, name))
        fields = parse_frontmatter(text) or {}
        if fields.get("doc-id"):
            documents[fields["doc-id"]] = (fields.get("status", ""), text)
    return documents


def check_matrix_coverage(matrix: dict, known: set[str]) -> None:
    gates = [gate for gate in matrix.get("gates") or [] if isinstance(gate, dict)]
    coverage = matrix.get("coverage") or {}
    if "criteriaVerified" in coverage:
        failures.append(
            f"{rel(MATRIX)} coverage still carries 'criteriaVerified'; ADR-0024 "
            f"decision 4 replaces it with criteriaClosed"
        )
    declared = sum(
        gate.get("criteria", 0) for gate in gates if isinstance(gate.get("criteria", 0), int)
    )
    if coverage.get("criteriaTotal") != declared:
        failures.append(
            f"{rel(MATRIX)} coverage.criteriaTotal={coverage.get('criteriaTotal')!r} "
            f"but gates[] sum to {declared}"
        )
    # The four spec status words are a partition; `closed` is deliberately not
    # one of them (ADR-0024 decisions 1 and 4).
    partition_keys = ("criteriaSpec", "criteriaProbe", "criteriaOpen", "criteriaWithdrawn")
    split = sum(coverage.get(key, 0) for key in partition_keys)
    if split != coverage.get("criteriaTotal"):
        failures.append(
            f"{rel(MATRIX)} coverage partitions into {split} criteria but declares "
            f"criteriaTotal={coverage.get('criteriaTotal')!r}"
        )
    # Gate ids (SPEC-GATE-A0) are aliases for a criterion family, so each must
    # own at least one defined criterion (SPEC-GATE-A0-01) in the spec.
    for gate in gates:
        gate_id = gate.get("id")
        if gate_id and not any(known_id.startswith(f"{gate_id}-") for known_id in known):
            failures.append(f"{rel(MATRIX)}: gate '{gate_id}' has no criterion under docs/")

    # The split must match the status column of docs/spec/08-gates.md, which is
    # where a criterion actually changes state; the matrix only projects it.
    criteria = spec_rows()
    if not criteria:
        failures.append(f"{rel(GATES)}: no SPEC-GATE-*-NN criterion rows found")
        return
    observed = Counter(row["status"] for row in criteria.values())
    # VERIFIED is a vocabulary assert, not a measurement: the spec layer never
    # claims it (08-gates.md:19, ADR-0012 decision 2, ADR-0024 decision 5).
    if observed.get("VERIFIED", 0):
        failures.append(
            f"{rel(GATES)}: {observed['VERIFIED']} criteria are VERIFIED; the spec "
            f"layer must never assert VERIFIED"
        )
    projected = {
        "SPEC": coverage.get("criteriaSpec", 0),
        "PROBE": coverage.get("criteriaProbe", 0),
        "OPEN": coverage.get("criteriaOpen", 0),
        "WITHDRAWN": coverage.get("criteriaWithdrawn", 0),
    }
    for status, count in projected.items():
        if observed.get(status, 0) != count:
            failures.append(
                f"{rel(MATRIX)} coverage says {count} {status} criteria but "
                f"{rel(GATES)} has {observed.get(status, 0)}"
            )
    if len(criteria) != coverage.get("criteriaTotal"):
        failures.append(
            f"{rel(MATRIX)} coverage.criteriaTotal={coverage.get('criteriaTotal')!r} "
            f"but {rel(GATES)} defines {len(criteria)} criteria"
        )
    for gate in gates:
        gate_id = gate.get("id")
        if not gate_id:
            continue
        actual = sum(1 for criterion in criteria if criterion.startswith(f"{gate_id}-"))
        if actual != gate.get("criteria"):
            failures.append(
                f"{rel(MATRIX)} gate '{gate_id}' declares {gate.get('criteria')} criteria "
                f"but {rel(GATES)} defines {actual}"
            )

    # closed is a derived count, never an input. It is a subset of PROBE.
    rows = [row for row in matrix.get("criteria") or [] if isinstance(row, dict)]
    stored_closed = sum(1 for row in rows if row.get("closed") is True)
    closed_count = coverage.get("criteriaClosed")
    if closed_count != stored_closed:
        failures.append(
            f"{rel(MATRIX)} coverage.criteriaClosed={closed_count!r} but criteria[] "
            f"holds {stored_closed} closed rows"
        )
    if not isinstance(closed_count, int) or closed_count > coverage.get("criteriaProbe", 0):
        failures.append(
            f"{rel(MATRIX)} coverage.criteriaClosed={closed_count!r} must be an integer "
            f"and at most criteriaProbe={coverage.get('criteriaProbe')!r}: closed ⊆ PROBE"
        )
    for gate in gates:
        gate_id = gate.get("id")
        if not gate_id:
            continue
        under = [row for row in rows if str(row.get("id", "")).startswith(f"{gate_id}-")]
        if not under:
            failures.append(
                f"{rel(MATRIX)} gate '{gate_id}' has no row in criteria[]"
            )
            continue
        expected = all(row.get("closed") is True for row in under)
        if "closed" not in gate:
            failures.append(f"{rel(MATRIX)} gate '{gate_id}' has no 'closed' field")
        elif gate.get("closed") is not expected:
            failures.append(
                f"{rel(MATRIX)} gate '{gate_id}' closed={gate.get('closed')!r} but its "
                f"criteria recompute to {expected}"
            )


def run_commit_known(run: dict, matrix: dict) -> bool:
    """ADR-0024: a run-level sha, or the matrix-level commit it inherits.

    An explicit `commit: null` is a recorded unknown, not an absence: it must
    never inherit the matrix-level commit, or a transcribed H1 run would look
    locatable and could back a closed obligation.
    """
    if "commit" in run:
        commit = run["commit"]
        return isinstance(commit, str) and bool(COMMIT_SHA.fullmatch(commit))
    return isinstance(matrix.get("commit"), str) and bool(COMMIT_SHA.fullmatch(matrix["commit"]))


def check_run_completeness(matrix: dict) -> None:
    """Every run states its exit, its test applicability, a summary and where
    its commit is known — or says explicitly that it is not.

    Silent key omission is how "11 runs without a commit, 21 without a tests
    count" stayed invisible. An explicit null with a note is a recorded fact;
    a missing key is not a record at all.
    """
    for index, run in enumerate(matrix.get("runs") or []):
        if not isinstance(run, dict):
            continue
        where = f"{rel(MATRIX)} runs[{index}] {run.get('id', '?')}"
        if "exit" not in run:
            failures.append(f"{where}: missing 'exit' (an int, or null with 'exitNote')")
        elif run["exit"] is None:
            if not run.get("exitNote"):
                failures.append(f"{where}: exit is null without an 'exitNote'")
        elif not isinstance(run["exit"], int) or isinstance(run["exit"], bool):
            failures.append(f"{where}: exit {run['exit']!r} is neither an int nor null")
        tests = run.get("tests")
        if not isinstance(tests, dict):
            failures.append(
                f"{where}: missing 'tests' object (an integer 'passed', or 'applicable': false)"
            )
        elif not isinstance(tests.get("passed"), int) and tests.get("applicable") is not False:
            failures.append(
                f"{where}: 'tests' has neither an integer 'passed' nor 'applicable': false"
            )
        if not (run.get("proves") or run.get("traceSha256")):
            failures.append(f"{where}: no summary — needs 'proves' or 'traceSha256'")
        if "commit" not in run:
            failures.append(
                f"{where}: missing 'commit' (a 40-character sha, or null with 'commitNote')"
            )
        elif run["commit"] is None and not run.get("commitNote"):
            failures.append(f"{where}: commit is null without a 'commitNote'")


def check_matrix_criteria(matrix: dict, known: set[str]) -> None:
    """Recompute ADR-0024's close predicate from criteria[].

    Three conditions make a criterion closed: its obligations partition both
    spec columns exactly, every obligation resolves (a #[test] name, a command
    quoted verbatim in validation.md, or a notApplicable transfer whose target
    text has landed), and every mapped obligation cites an exit-0 run with a
    locatable commit and a summary. The stored boolean is only compared with
    that recomputation — it is never trusted.
    """
    spec = spec_rows()
    rows = matrix.get("criteria")
    if not isinstance(rows, list) or not rows:
        failures.append(
            f"{rel(MATRIX)}: 'criteria' must be a non-empty list (ADR-0024 decision 3)"
        )
        return
    tests = test_manifest()
    commands = recorded_commands(read_text(VALIDATION))
    adrs = adr_documents()
    runs = {
        run.get("id"): run
        for run in matrix.get("runs") or []
        if isinstance(run, dict) and isinstance(run.get("id"), str)
    }

    def ref_resolves(ref: object) -> bool:
        if not isinstance(ref, str) or not ref.strip():
            return False
        return ref in tests or ref in commands

    def adr_resolves(adr: object, source: str) -> bool:
        if not isinstance(adr, str) or re.fullmatch(r"ADR-\d{4}", adr) is None:
            return False
        if adr not in known or adr not in adrs:
            return False
        status, body = adrs[adr]
        short = source.split("SPEC-GATE-", 1)[-1]  # ADR-0025 writes "A0-03"
        # The ADR must be about transfers and must name this criterion. Several
        # ADRs mention A0-11 as a counter-argument, so naming alone is not it.
        # ADR-0024 defines the predicate and ADR-0026 only corrects ADR-0025:
        # neither is a handover, so neither may authorize a transfer.
        return (
            status == "normative"
            and adr not in {"ADR-0024", "ADR-0026"}
            and "notApplicable" in body
            and "acceptedBy" in body
            and (source in body or short in body)
        )

    def accepted_lands(accepted: object, source: str) -> bool:
        """The target must be a criterion in another gate, and the text must
        already be in one of that criterion's two columns — not straddling them.
        """
        if not isinstance(accepted, dict):
            return False
        target_id = accepted.get("criterion")
        if not isinstance(target_id, str):
            return False
        target = spec.get(target_id)
        text = accepted.get("text")
        if target is None or target_id == source or target["gate"] == spec[source]["gate"]:
            return False
        if not isinstance(text, str):
            return False
        needle = strip_text(text)
        # A transfer must name an obligation, not a particle: the verbatim
        # substring test would otherwise pass on "的".
        if len(needle) < 6:
            return False
        return needle in strip_text(target["must"]) or needle in strip_text(target["evidence"])

    def expected_failure_ok(run_id: object) -> bool:
        run = runs.get(run_id) if isinstance(run_id, str) else None
        return (
            isinstance(run, dict)
            and run.get("kind") == "compile-fail"
            and isinstance(run.get("exit"), int)
            and run["exit"] != 0
            and run_commit_known(run, matrix)
            and bool(run.get("proves") or run.get("traceSha256"))
        )

    def backed(obligation: dict, criterion_evidence: list) -> bool:
        ids = obligation["evidence"] if "evidence" in obligation else criterion_evidence
        if not isinstance(ids, list):
            return False
        for run_id in ids:
            run = runs.get(run_id) if isinstance(run_id, str) else None
            if (
                isinstance(run, dict)
                and run.get("exit") == 0
                and run_commit_known(run, matrix)
                and (run.get("proves") or run.get("traceSha256"))
            ):
                return True
        return False

    def obligation_ok(obligation: dict, criterion_evidence: list, source: str) -> bool:
        if not isinstance(obligation, dict):
            return False
        not_applicable = obligation.get("kind") == "notApplicable"
        ok = True
        if not_applicable:
            ok = adr_resolves(obligation.get("adr"), source) and accepted_lands(
                obligation.get("acceptedBy"), source
            )
        # A partially transferred obligation keeps its ref; a fully transferred
        # one has none. Either way, every ref that is present must resolve and
        # be backed — expectedFailure is an extra constraint, not a way out.
        if "ref" in obligation:
            if not ref_resolves(obligation.get("ref")):
                ok = False
            if not backed(obligation, criterion_evidence):
                ok = False
        if "expectedFailure" in obligation:
            # An expected-failure obligation still needs a fixture name; the
            # run alone is not a witness for "the fixture exists".
            if "ref" not in obligation:
                ok = False
            elif not expected_failure_ok(obligation.get("expectedFailure")):
                ok = False
        if "ref" not in obligation and "expectedFailure" not in obligation and not not_applicable:
            ok = False
        return ok

    by_id: dict[str, dict] = {}
    for index, row in enumerate(rows):
        where = f"{rel(MATRIX)} criteria[{index}]"
        if not isinstance(row, dict):
            failures.append(f"{where}: must be an object")
            continue
        criterion = row.get("id")
        if criterion not in spec:
            failures.append(f"{where}: {criterion!r} is not defined in {rel(GATES)}")
            continue
        if criterion in by_id:
            failures.append(f"{where}: duplicate criterion '{criterion}'")
            continue
        by_id[criterion] = row
        if row.get("gate") != spec[criterion]["gate"]:
            failures.append(
                f"{where} ({criterion}): gate {row.get('gate')!r} but the id belongs to "
                f"'{spec[criterion]['gate']}'"
            )
        if row.get("status") != spec[criterion]["status"]:
            failures.append(
                f"{where} ({criterion}): status {row.get('status')!r} but {rel(GATES)} "
                f"says {spec[criterion]['status']!r}"
            )
        obligations = row.get("obligations")
        if not isinstance(obligations, list):
            failures.append(f"{where} ({criterion}): 'obligations' must be a list")
            obligations = []
        for obligation in obligations:
            if not isinstance(obligation, dict):
                failures.append(f"{where} ({criterion}): obligation is not an object")
                continue
            if obligation.get("source") not in OBLIGATION_SOURCES:
                failures.append(
                    f"{where} ({criterion}): obligation source "
                    f"{obligation.get('source')!r} must be one of {OBLIGATION_SOURCES}"
                )
            if not isinstance(obligation.get("text"), str):
                failures.append(f"{where} ({criterion}): obligation text must be a string")
            if obligation.get("kind") == "notApplicable":
                if not adr_resolves(obligation.get("adr"), criterion):
                    failures.append(
                        f"{where} ({criterion}): notApplicable obligation without a "
                        f"defined ADR ({obligation.get('adr')!r}); ADR-0024 requires one"
                    )
                if not accepted_lands(obligation.get("acceptedBy"), criterion):
                    failures.append(
                        f"{where} ({criterion}): acceptedBy target or text has not "
                        f"landed ({obligation.get('acceptedBy')!r})"
                    )
            if "expectedFailure" in obligation and not isinstance(
                runs.get(obligation.get("expectedFailure")), dict
            ):
                failures.append(
                    f"{where} ({criterion}): expectedFailure "
                    f"{obligation.get('expectedFailure')!r} is not a declared run"
                )
        for source, column in (
            ("criterion", spec[criterion]["must"]),
            ("evidence", spec[criterion]["evidence"]),
        ):
            joined = "".join(
                str(obligation.get("text", ""))
                for obligation in obligations
                if isinstance(obligation, dict) and obligation.get("source") == source
            )
            if strip_text(joined) != strip_text(column):
                failures.append(
                    f"{where} ({criterion}): {source} obligations do not concatenate "
                    f"to the {rel(GATES)} column ({len(strip_text(joined))} chars vs "
                    f"{len(strip_text(column))} expected); every character must be mapped"
                )
        if "evidence" not in row:
            failures.append(f"{where} ({criterion}): missing 'evidence' list")
            evidence = row.get("evidence") or []
        evidence = row.get("evidence")
        if not isinstance(evidence, list):
            failures.append(f"{where} ({criterion}): 'evidence' must be a list")
            evidence = []
        for run_id in evidence:
            if not isinstance(run_id, str) or run_id not in runs:
                failures.append(
                    f"{where} ({criterion}): evidence cites unknown run {run_id!r}"
                )
        computed = (
            row.get("status") == "PROBE"
            and bool(obligations)
            and all(obligation_ok(obligation, evidence, criterion) for obligation in obligations)
        )
        if row.get("closed") is not computed:
            failures.append(
                f"{where} ({criterion}): stored closed={row.get('closed')!r} but the "
                f"predicate recomputes to {computed}"
            )
    for missing in sorted(set(spec) - set(by_id)):
        failures.append(f"{rel(MATRIX)}: criteria[] is missing {missing}")


def check_matrix_matches_validation(matrix: dict) -> None:
    """matrix.json and validation.md §2 must describe the same runs."""
    index: dict[str, tuple[str, str]] = {}
    for line in read_text(VALIDATION).splitlines():
        m = VALIDATION_ROW.match(line)
        if not m:
            continue
        cells = [cell.strip("*").strip() for cell in split_row(line)]
        if len(cells) < 4:
            failures.append(f"{rel(VALIDATION)}: malformed index row for {m.group(1)}")
            continue
        index[m.group(1)] = (cells[1], cells[3])

    runs = {
        run["id"]: (run.get("class"), run.get("host"))
        for run in matrix.get("runs") or []
        if isinstance(run, dict) and isinstance(run.get("id"), str)
    }
    for run_id in sorted(set(index) | set(runs)):
        if run_id not in runs:
            failures.append(f"{rel(MATRIX)}: {run_id} is in validation.md but has no run")
        elif run_id not in index:
            failures.append(f"{rel(VALIDATION)}: {run_id} is in matrix.json but not in §2")
        elif index[run_id] != runs[run_id]:
            failures.append(
                f"{run_id}: validation.md §2 says class/host {index[run_id]} "
                f"but matrix.json says {runs[run_id]}"
            )


def git(*argv: str) -> tuple[bool, str]:
    """Run git against the corpus checkout; returns (ok, stderr)."""
    try:
        result = subprocess.run(
            ["git", "-C", ROOT, *argv],
            capture_output=True,
            text=True,
            timeout=60,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        return False, str(exc)
    return result.returncode == 0, result.stderr.strip()


def check_run_commits(matrix: dict) -> None:
    """Every commit a run cites must exist here and be an ancestor of HEAD.

    A rebase rewrites hashes. The entry keeps its id, its numbers and its
    `commit` string, but the object is gone — and nothing in the text can tell
    that apart from an honest record. Only the repository can. Runs without a
    `commit` field (transcribed hosts, external fixtures) are skipped: there is
    nothing to look up, and inventing one would be worse than the gap. A null
    field is that gap made explicit: check_run_completeness requires a
    `commitNote` beside it, and no obligation may rely on such a run. A field
    that is empty or malformed is not the gap — it is bad data.
    """
    runs = [run for run in matrix.get("runs") or [] if isinstance(run, dict)]
    cited = [(run.get("id"), run["commit"]) for run in runs if "commit" in run]
    if not cited:
        return
    inside, stderr = git("rev-parse", "--is-inside-work-tree")
    if not inside:
        failures.append(
            f"{rel(MATRIX)}: cannot verify commit provenance — not a git checkout "
            f"({stderr or 'git unavailable'})"
        )
        return
    for run_id, commit in cited:
        if commit is None:
            # Explicit unknown, paired with a commitNote by check_run_completeness.
            continue
        if not isinstance(commit, str) or not COMMIT_SHA.fullmatch(commit):
            failures.append(
                f"{rel(MATRIX)} {run_id}: commit {commit!r} is not a full "
                f"40-character sha; an empty or malformed value is bad data, not a "
                f"missing record — use null plus a commitNote, or drop the field"
            )
            continue
        exists, _ = git("cat-file", "-e", f"{commit}^{{commit}}")
        if not exists:
            failures.append(
                f"{rel(MATRIX)} {run_id}: commit {commit} does not exist in this "
                f"repository (rewritten, never pushed, or the clone is shallow — "
                f"the docs job checks out with fetch-depth: 0)"
            )
            continue
        ancestor, _ = git("merge-base", "--is-ancestor", commit, "HEAD")
        if not ancestor:
            failures.append(
                f"{rel(MATRIX)} {run_id}: commit {commit} exists but is not an "
                f"ancestor of HEAD (another branch, or a rebuild that dropped it)"
            )


def validation_entries() -> tuple[set[str], set[str]]:
    """The EVID-VAL ids the validation ledger declares: §2 index rows and the
    entry sections themselves. Grouped headings ("EVID-VAL-09 / -10") count
    for every id they name."""
    text = read_text(VALIDATION)
    index = {m.group(1) for m in (VALIDATION_ROW.match(line) for line in text.splitlines()) if m}
    headings: set[str] = set()
    for line in text.splitlines():
        if not re.match(r"^#{2,6}\s", line):
            continue
        for match in re.finditer(r"EVID-VAL-(\d{2})", line):
            headings.add(f"EVID-VAL-{match.group(1)}")
        for match in re.finditer(r"EVID-VAL-\d{2}\s*/\s*-(\d{2})", line):
            headings.add(f"EVID-VAL-{match.group(1)}")
    return index, headings


def highest_validation_id() -> int:
    index, headings = validation_entries()
    for missing in sorted(headings - index):
        failures.append(f"{rel(VALIDATION)}: entry section {missing} has no §2 index row")
    for extra in sorted(index - headings):
        failures.append(f"{rel(VALIDATION)}: §2 lists {extra} but no entry section defines it")
    if not index:
        failures.append(f"{rel(VALIDATION)}: no EVID-VAL rows found")
        return 0
    return max(int(entry.rsplit("-", 1)[1]) for entry in index)


def check_derived_numbers(matrix: dict, max_val: int) -> None:
    """Recompute every number the entry documents quote."""
    coverage = matrix.get("coverage") or {}
    expected_coverage = (
        coverage.get("criteriaTotal"),
        coverage.get("criteriaClosed"),
        coverage.get("criteriaProbe"),
        coverage.get("criteriaOpen"),
    )
    derived = matrix_counts(matrix)

    for name in ENTRY_DOCUMENTS:
        path = os.path.join(ROOT, name)
        if not os.path.exists(path):
            failures.append(f"{name}: missing entry document")
            continue
        coverage_lines: list[int] = []
        for line_no, line in enumerate(read_text(path).splitlines(), 1):
            mark = COVERAGE_MARK.search(line)
            if mark:
                coverage_lines.append(line_no)
                labels: dict[str, int] = {}
                for count, status in COVERAGE_LABEL.findall(line):
                    if status in labels:
                        failures.append(
                            f"{name}:{line_no}: coverage sentence repeats the "
                            f"{status} label"
                        )
                    labels[status] = int(count)
                closed_match = COVERAGE_CLOSED.search(line)
                if "VERIFIED" in labels:
                    failures.append(
                        f"{name}:{line_no}: coverage sentence still quotes VERIFIED; "
                        f"ADR-0024 decision 5 stops that usage"
                    )
                quoted = (
                    int(mark.group(1)),
                    int(closed_match.group(1)) if closed_match else None,
                    labels.get("PROBE"),
                    labels.get("OPEN"),
                )
                if None in quoted:
                    failures.append(
                        f"{name}:{line_no}: coverage sentence must name the total, the "
                        f"closed count (the '其中 N closed' form), PROBE and OPEN"
                    )
                elif quoted != expected_coverage:
                    failures.append(
                        f"{name}:{line_no}: coverage {quoted} does not match "
                        f"EVID-MATRIX.coverage {expected_coverage}"
                    )
            for match in EVID_RANGE.finditer(line):
                if int(match.group(1)) != max_val:
                    failures.append(
                        f"{name}:{line_no}: EVID-VAL-01…{match.group(1)} but validation.md "
                        f"defines up to EVID-VAL-{max_val:02d}"
                    )
            for match in KEY_VALUE.finditer(line):
                key, value = match.group(1), int(match.group(2))
                if key in derived and derived[key] != value:
                    failures.append(
                        f"{name}:{line_no}: {key}={value} but runs[] recompute to {derived[key]}"
                    )
        if len(coverage_lines) != 1:
            failures.append(
                f"{name}: expected exactly one coverage sentence, found "
                f"{len(coverage_lines)} (lines {coverage_lines})"
            )


def check_claim_totals() -> None:
    text = read_text(CLAIMS)
    rows: list[tuple[str, str]] = []
    for line in text.splitlines():
        m = CLAIM_ROW.match(line)
        if not m:
            continue
        cells = split_row(line)
        if len(cells) < 5:
            continue  # malformed rows are reported by check_claim_evidence
        rows.append((m.group(1), cells[2]))

    if not rows:
        failures.append(f"{rel(CLAIMS)}: no CLAIM rows found")
        return
    ids = [claim_id for claim_id, _ in rows]
    duplicates = sorted(claim_id for claim_id, count in Counter(ids).items() if count > 1)
    if duplicates:
        failures.append(f"{rel(CLAIMS)}: duplicate claim rows {duplicates}")
    expected = [f"CLAIM-{n:03d}" for n in range(1, len(ids) + 1)]
    if sorted(ids) != expected:
        failures.append(
            f"{rel(CLAIMS)}: claim ids must be contiguous CLAIM-001…{len(ids):03d}"
        )

    for status in set(status for _, status in rows) - set(STATUS_WORDS):
        failures.append(f"{rel(CLAIMS)}: unknown claim status '{status}'")

    derived = Counter(status for _, status in rows)
    summary: dict[str, int] = {}
    total = None
    for line in text.splitlines():
        m = CLAIM_SUMMARY_ROW.match(line)
        if m:
            summary[m.group(1)] = int(m.group(2))
        m = CLAIM_TOTAL_ROW.match(line)
        if m:
            total = int(m.group(1))
    if total != len(rows):
        failures.append(f"{rel(CLAIMS)} §2: total {total} but the ledger has {len(rows)} rows")
    for status in STATUS_WORDS:
        if summary.get(status, 0) != derived.get(status, 0):
            failures.append(
                f"{rel(CLAIMS)} §2: {status} counts {summary.get(status, 0)} "
                f"but the ledger has {derived.get(status, 0)}"
            )


def check_claim_evid_keys() -> None:
    """The evidence-key table must cover every EVID-VAL in validation.md."""
    keys = {
        m.group(1)
        for m in (VALIDATION_ROW.match(line) for line in read_text(CLAIMS).splitlines())
        if m
    }
    defined = {
        m.group(1)
        for m in (VALIDATION_ROW.match(line) for line in read_text(VALIDATION).splitlines())
        if m
    }
    for missing in sorted(defined - keys):
        failures.append(f"{rel(CLAIMS)} §0.1: evidence key table is missing {missing}")
    for extra in sorted(keys - defined):
        failures.append(
            f"{rel(CLAIMS)} §0.1: evidence key table lists {extra}, "
            f"which {rel(VALIDATION)} does not define"
        )


def main() -> int:
    files = markdown_files()
    check_frontmatter(files)
    check_template_residue(files)
    check_links(files)
    known = defined_ids(files)
    check_ids(files, known)
    check_duplicate_ids(files)
    check_claim_evidence(files)
    matrix = load_matrix()
    max_val = highest_validation_id()
    if matrix is not None:
        check_matrix_runs(matrix)
        check_run_completeness(matrix)
        check_matrix_counts(matrix)
        check_matrix_coverage(matrix, known)
        check_matrix_criteria(matrix, known)
        check_matrix_matches_validation(matrix)
        check_run_commits(matrix)
        check_derived_numbers(matrix, max_val)
        check_claim_totals()
        check_claim_evid_keys()
    if failures:
        for item in failures:
            print(item)
        print(f"\n{len(failures)} problem(s) in {len(files)} Markdown files")
        return min(len(failures), 99)
    print(f"docs check passed: {len(files)} Markdown files")
    return 0


if __name__ == "__main__":
    sys.exit(main())

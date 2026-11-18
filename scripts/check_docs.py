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
     and the claim-register evidence-key table covers every EVID-VAL.

Exit code is the number of failures, capped at 99. Standard library only.
"""

from __future__ import annotations

import json
import os
import re
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
GATE_ROW = re.compile(r"^\|\s*`?(SPEC-GATE-[A-Z0-9]+-\d{2})`?\s*\|")
GATE_STATUS = re.compile(r"^`(VERIFIED|PROBE|OPEN|SPEC|WITHDRAWN)`")
DERIVED_KEYS = ("runsTotal", "testsPassedH2", "runsReproducibleHere", "runsTranscribedOrExternal")
KEY_VALUE = re.compile(r"\b(" + "|".join(DERIVED_KEYS) + r")\s*=\s*(\d+)")

RUN_REQUIRED = ("id", "class", "host", "kind", "command")
RUN_CLASSES = {"A", "B", "C", "D"}

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


def spec_criteria() -> dict[str, str]:
    """Criterion id -> status, read from the status cell of the gate table."""
    criteria: dict[str, str] = {}
    for line in read_text(GATES).splitlines():
        m = GATE_ROW.match(line)
        if not m:
            continue
        cells = split_row(line)
        status = GATE_STATUS.match(cells[-1]) if cells else None
        if not status:
            failures.append(f"{rel(GATES)}: {m.group(1)} has no status word in its last cell")
            continue
        criteria[m.group(1)] = status.group(1)
    return criteria


def check_matrix_coverage(matrix: dict, known: set[str]) -> None:
    gates = [gate for gate in matrix.get("gates") or [] if isinstance(gate, dict)]
    coverage = matrix.get("coverage") or {}
    declared = sum(
        gate.get("criteria", 0) for gate in gates if isinstance(gate.get("criteria", 0), int)
    )
    if coverage.get("criteriaTotal") != declared:
        failures.append(
            f"{rel(MATRIX)} coverage.criteriaTotal={coverage.get('criteriaTotal')!r} "
            f"but gates[] sum to {declared}"
        )
    split = sum(
        coverage.get(key, 0) for key in ("criteriaVerified", "criteriaProbe", "criteriaOpen")
    )
    if split != coverage.get("criteriaTotal"):
        failures.append(
            f"{rel(MATRIX)} coverage splits into {split} criteria but declares "
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
    criteria = spec_criteria()
    if not criteria:
        failures.append(f"{rel(GATES)}: no SPEC-GATE-*-NN criterion rows found")
        return
    observed = Counter(criteria.values())
    projected = {
        "VERIFIED": coverage.get("criteriaVerified", 0),
        "PROBE": coverage.get("criteriaProbe", 0),
        "OPEN": coverage.get("criteriaOpen", 0),
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
        coverage.get("criteriaVerified"),
        coverage.get("criteriaProbe"),
        coverage.get("criteriaOpen"),
    )
    derived = matrix_counts(matrix)

    for name in ENTRY_DOCUMENTS:
        path = os.path.join(ROOT, name)
        if not os.path.exists(path):
            failures.append(f"{name}: missing entry document")
            continue
        for line_no, line in enumerate(read_text(path).splitlines(), 1):
            mark = COVERAGE_MARK.search(line)
            if mark:
                labels = {status: int(count) for count, status in COVERAGE_LABEL.findall(line)}
                quoted = (
                    int(mark.group(1)),
                    labels.get("VERIFIED"),
                    labels.get("PROBE"),
                    labels.get("OPEN"),
                )
                if None in quoted:
                    failures.append(
                        f"{name}:{line_no}: coverage sentence names the total but not "
                        f"VERIFIED/PROBE/OPEN counts"
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
    check_links(files)
    known = defined_ids(files)
    check_ids(files, known)
    check_duplicate_ids(files)
    check_claim_evidence(files)
    matrix = load_matrix()
    max_val = highest_validation_id()
    if matrix is not None:
        check_matrix_runs(matrix)
        check_matrix_counts(matrix)
        check_matrix_coverage(matrix, known)
        check_matrix_matches_validation(matrix)
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

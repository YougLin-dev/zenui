#!/usr/bin/env python3
"""Check relative links in the Markdown files changed by a push or pull request.

The workspace workflow passes the JSON array produced by dorny/paths-filter as
the only argument. Anchors and external URLs are not resolved here.
"""

import json
import pathlib
import re
import sys

LINK = re.compile(r"\]\(([^)]+)\)")
SKIP = re.compile(r"(?:https?:|mailto:|#)")


def changed_markdown(argument: str) -> list[pathlib.Path]:
    names = json.loads(argument)
    if not isinstance(names, list) or not all(isinstance(name, str) for name in names):
        raise ValueError("expected a JSON array of paths")
    paths = [pathlib.Path(name) for name in names]
    return [path for path in paths if path.suffix == ".md" and path.exists()]


def broken_links(path: pathlib.Path) -> list[tuple[pathlib.Path, str]]:
    broken = []
    for target in LINK.findall(path.read_text(encoding="utf-8")):
        if SKIP.match(target):
            continue
        relative = target.split("#", 1)[0]
        if relative and not (path.parent / relative).exists():
            broken.append((path, target))
    return broken


def main() -> int:
    try:
        files = changed_markdown(sys.argv[1])
    except (IndexError, json.JSONDecodeError, ValueError) as error:
        print(f"cannot read changed docs: {error}")
        return 1
    broken = [entry for path in dict.fromkeys(files) for entry in broken_links(path)]
    for path, target in broken:
        print(f"{path}: broken link {target}")
    return 1 if broken else 0


if __name__ == "__main__":
    raise SystemExit(main())

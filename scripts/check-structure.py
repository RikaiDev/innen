#!/usr/bin/env python3
"""Structural gates: single responsibility and task-marker hygiene.

These are the defects review kept missing: one file holding several unrelated
jobs, a second hand-rolled parser beside the first, and internal "Task 12"
markers leaking into user-facing `--help` text.

Refusals (exit 1) are for regressions this gate exists to catch. A file that
was already over budget when the gate landed is listed in
`scripts/structure-baseline.json` as debt to split, not as an exemption: a
listed file that no longer exists, or that has dropped under budget, is itself
a failure, so the debt cannot silently outlive its own fix.

The baseline is parsed as JSON, not grepped. An earlier version grepped it and
therefore shipped a malformed baseline unnoticed.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
BASELINE = REPO / "scripts" / "structure-baseline.json"

# Roots scanned for the line budget, each with the depth that separates a
# module file from a submodule directory.
SOURCE_ROOTS = {"src": 2, "crates/innen-core/src": 3}

# `Task` is also a NodeType and a middleware task; only planning markers
# ("Task 12", "Task 8c") are planning vocabulary with no meaning to a reader.
TASK_MARKER = re.compile(r"\bTask\s+\d+[a-z]?\b")
TASK_MARKER_ALLOWED = (
    '"type": *"Task"',
    "NodeType::Task",
    '"Task" =>',
    "Task status is recorded",
)

failures: list[str] = []


def fail(message: str) -> None:
    failures.append(message)


def source_files() -> list[pathlib.Path]:
    """Every module file under the scanned roots, skipping vendored code."""
    found: list[pathlib.Path] = []
    for root, depth in SOURCE_ROOTS.items():
        base = REPO / root
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            if len(path.relative_to(base).parts) > depth:
                continue
            found.append(path)
    return found


def check_line_budget(files: list[pathlib.Path]) -> None:
    if not BASELINE.is_file():
        for path in files:
            lines = len(path.read_text().splitlines())
            if lines > 400:
                fail(f"{rel(path)}: {lines} lines exceeds the 400-line budget")
        return

    try:
        baseline = json.loads(BASELINE.read_text())
    except json.JSONDecodeError as error:
        fail(f"{rel(BASELINE)}: malformed JSON ({error}); the gate cannot trust it")
        return

    budget = baseline.get("max_lines", 400)
    listed = {entry["file"] for entry in baseline.get("over_budget", [])}

    still_over: set[str] = set()
    for path in files:
        name = rel(path)
        lines = len(path.read_text().splitlines())
        if lines > budget:
            still_over.add(name)
            if name not in listed:
                fail(f"{name}: {lines} lines exceeds {budget} and has no baseline entry")

    for name in sorted(listed):
        if name not in still_over:
            if not (REPO / name).is_file():
                fail(f"{name}: baseline entry no longer exists; remove it")
            else:
                fail(f"{name}: baseline entry is no longer over {budget}; remove it")


def check_task_markers(files: list[pathlib.Path]) -> None:
    for path in files:
        for number, line in enumerate(path.read_text().splitlines(), start=1):
            if not TASK_MARKER.search(line):
                continue
            if any(allowed in line for allowed in TASK_MARKER_ALLOWED):
                continue
            fail(f"{rel(path)}:{number}: internal task marker: {line.strip()}")


def rel(path: pathlib.Path) -> str:
    return str(path.relative_to(REPO))


def main() -> int:
    files = source_files()
    if not files:
        print("structure: no source files discovered", file=sys.stderr)
        return 1
    check_line_budget(files)
    check_task_markers(files)

    if failures:
        for message in failures:
            print(f"structure: {message}", file=sys.stderr)
        return 1
    print(f"structure: ok ({len(files)} files, line budget, task markers)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

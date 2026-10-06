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
import subprocess
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


def committed_lines(rel: str) -> int | None:
    """Line count of `rel` as committed, or None outside a repository.

    The debt list must describe the repository, not whatever another session
    happens to have in its working tree. Measured from a dirty tree once, this
    recorded a 384-line committed file as 530 because a concurrent session was
    mid-refactor, and CI rejected the result.
    """
    result = subprocess.run(
        ["git", "show", f"HEAD:{rel}"], capture_output=True, text=True
    )
    if result.returncode != 0:
        return None
    return len(result.stdout.splitlines())


def write_baseline(files: list[pathlib.Path]) -> int:
    """Regenerate the baseline from committed content."""
    entries = []
    for path in files:
        rel = str(path.relative_to(REPO))
        lines = committed_lines(rel)
        if lines is None:
            lines = len(path.read_text().splitlines())
        if lines > 400:
            entries.append({"file": rel, "lines": lines, "split_by": "TODO"})
    entries.sort(key=lambda entry: -entry["lines"])
    BASELINE.write_text(
        json.dumps(
            {
                "comment": (
                    "Files over the 400-line budget, measured from committed content. Debt "
                    "to split by single responsibility, not an exemption: remove an entry "
                    "when the file is split or drops under budget. The gate fails on a stale "
                    "entry, an unlisted oversized file, and a malformed baseline. Regenerate "
                    "with `python3 scripts/check-structure.py --write-baseline`, which measures "
                    "HEAD so another session's uncommitted work cannot move this list."
                ),
                "max_lines": 400,
                "over_budget": entries,
            },
            indent=2,
        )
        + "\n"
    )
    return len(entries)


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
        # Measured from HEAD, like the baseline. A concurrent session's
        # uncommitted refactor must not appear here as new debt, and must not
        # appear in the baseline either: the two must describe the same
        # repository or the gate contradicts itself.
        lines = committed_lines(name)
        if lines is None:
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


def main(argv: list[str]) -> int:
    files = source_files()
    if not files:
        print("structure: no source files discovered", file=sys.stderr)
        return 1
    if "--write-baseline" in argv:
        count = write_baseline(files)
        print(f"structure: baseline rewritten with {count} entries (from HEAD)")
        return 0
    check_line_budget(files)
    check_task_markers(files)

    if failures:
        for message in failures:
            print(f"structure: {message}", file=sys.stderr)
        return 1
    print(f"structure: ok ({len(files)} files, line budget, task markers)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

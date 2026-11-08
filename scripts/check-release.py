#!/usr/bin/env python3
"""Fail-closed release/documentation harness.

Single sources of truth:
- versions: Cargo.toml [package].version
- release notes: CHANGELOG.md `## [X.Y.Z]` sections
- tags: git tags `vX.Y.Z` (checked when present/required by caller)
- docs: no per-version `docs/release-v*.md`; README must not link to them
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
CHANGELOG_HEADING = re.compile(r"^## \[(\d+\.\d+\.\d+)\](?:\s+-\s+.*)?\s*$", re.M)
RELEASE_DOC = re.compile(r"^release-v\d+\.\d+\.\d+\.md$")


def fail(msg: str) -> None:
    print(f"release: {msg}", file=sys.stderr)
    raise SystemExit(1)


def cargo_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    in_pkg = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            in_pkg = s == "[package]"
        if in_pkg and s.startswith("version"):
            m = re.search(r'"([^"]+)"', s)
            if m:
                return m.group(1)
    fail("Cargo.toml [package].version not found")
    return ""


def changelog_versions() -> list[str]:
    text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    return CHANGELOG_HEADING.findall(text)


def git_tags() -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(ROOT), "tag", "--list", "v*"],
        check=False,
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        return []
    return [t for t in out.stdout.splitlines() if TAG.match(t)]


def check_docs() -> list[str]:
    errs: list[str] = []
    docs = ROOT / "docs"
    if docs.is_dir():
        for p in docs.iterdir():
            if p.is_file() and RELEASE_DOC.match(p.name):
                errs.append(
                    f"{p.relative_to(ROOT)}: per-version release docs are removed; use CHANGELOG.md"
                )
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    for m in re.finditer(r"docs/release-v\d+\.\d+\.\d+\.md", readme):
        errs.append(f"README.md links to removed {m.group(0)}; use CHANGELOG.md")
    return errs


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--require-tag", action="store_true", help="require git tag v<Cargo version>")
    ap.add_argument("--strict-tags", action="store_true", help="require CHANGELOG entry for every v* tag")
    ap.add_argument("--format", choices=["json", "text"], default="text")
    args = ap.parse_args()

    errs = check_docs()
    version = cargo_version()
    if not SEMVER.match(version):
        errs.append(f"Cargo.toml version {version!r} is not semver X.Y.Z")
    ch = changelog_versions()
    if not ch:
        errs.append("CHANGELOG.md has no `## [X.Y.Z]` headings")
    elif ch[0] != version:
        errs.append(
            f"CHANGELOG latest [{ch[0]}] != Cargo.toml [{version}] (bump and record before release)"
        )
    tags = git_tags()
    tag_names = {t[1:] for t in tags}
    if args.require_tag and version not in tag_names:
        errs.append(f"git tag v{version} missing")
    if args.strict_tags:
        for t in sorted(tag_names):
            if t not in ch:
                errs.append(f"git tag v{t} has no CHANGELOG entry")

    report = {
        "ok": not errs,
        "cargo_version": version,
        "changelog_latest": ch[0] if ch else None,
        "errors": errs,
    }
    if args.format == "json":
        print(json.dumps(report, ensure_ascii=False, indent=2))
    elif errs:
        for e in errs:
            print(f"release: {e}", file=sys.stderr)
    else:
        print(f"release: ok (version {version}, changelog aligned, docs clean)")
    return 1 if errs else 0


if __name__ == "__main__":
    raise SystemExit(main())

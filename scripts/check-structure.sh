#!/usr/bin/env bash
# Structural gates for single responsibility and task-reference hygiene.
#
# These are the defects that review kept missing: a 664-line `parity.rs` holding
# five unrelated jobs, a second hand-rolled TOML parser beside the first, and
# internal "Task 12" markers leaking into user-facing `--help` text.
#
# Refusals (exit 1) are for regressions this gate is meant to catch. A finding
# that predates the gate belongs in scripts/structure-baseline.json with a
# removal condition, not in a loosened limit.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

baseline=scripts/structure-baseline.json
max_lines=400
status=0

fail() { printf 'structure: %s\n' "$1" >&2; status=1; }

# --- Gate 1: no source file may exceed the line budget. -------------------
# Files already over budget at the time this gate landed are listed in the
# baseline and must shrink; they are not a permanent exemption.
over_budget=()
while IFS= read -r -d '' file; do
  lines="$(wc -l <"$file" | tr -d ' ')"
  if [ "$lines" -gt "$max_lines" ]; then
    over_budget+=("$file:$lines")
  fi
done < <(find src -maxdepth 2 -name '*.rs' -print0; find crates/innen-core/src -maxdepth 1 -name '*.rs' -print0)

if [ "${#over_budget[@]}" -gt 0 ]; then
  if [ -f "$baseline" ]; then
    for entry in "${over_budget[@]}"; do
      file="${entry%:*}"
      if ! grep -q "\"$file\"" "$baseline"; then
        fail "$entry exceeds ${max_lines} lines and has no baseline entry"
      fi
    done
  else
    for entry in "${over_budget[@]}"; do
      fail "$entry exceeds ${max_lines} lines"
    done
  fi
fi

# --- Gate 2: no internal task markers in code or user-facing text. -------
# "Task 12" is planning vocabulary with no meaning to a reader of the code.
if grep -rnE '\bTask[[:space:]]+[0-9]+[a-z]?\b' \
     --include='*.rs' src crates/innen-core/src \
     --exclude-dir=target 2>/dev/null | grep -v '"type": *"Task"' \
     | grep -v 'NodeType::Task' | grep -v '"Task" =>' | grep -v 'Task status is recorded' \
     >/dev/null; then
  grep -rnE '\bTask[[:space:]]+[0-9]+[a-z]?\b' \
       --include='*.rs' src crates/innen-core/src 2>/dev/null \
    | grep -v '"type": *"Task"' | grep -v 'NodeType::Task' \
    | grep -v '"Task" =>' | grep -v 'Task status is recorded' \
    | sed 's/^/structure: internal task marker: /' >&2
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo 'structure: ok (single-file budget, task markers)'
fi
exit "$status"

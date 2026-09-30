#!/usr/bin/env bash
# Final release step: install the official release through Homebrew, then retire
# the dev build in ~/.local/bin once the release provably contains it.
# Usage: scripts/post-release.sh vX.Y.Z [--wait-seconds N]
set -euo pipefail
tag="${1:?usage: post-release.sh vX.Y.Z [--wait-seconds N]}"
wait_seconds=600
if [ "${2:-}" = "--wait-seconds" ]; then wait_seconds="${3:?}"; fi
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag must look like vX.Y.Z" >&2; exit 2; }
version="${tag#v}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
formula="rikaidev/tap/innen"
bin_dir="${INNEN_DEV_BIN_DIR:-$HOME/.local/bin}"
marker="${INNEN_DEV_MARKER_DIR:-$HOME/.local/share/innen}/dev-install.json"

deadline=$(( $(date +%s) + wait_seconds ))
until brew update --quiet >/dev/null 2>&1; [ "$(brew info --json=v2 "$formula" | python3 -c 'import json,sys;print(json.load(sys.stdin)["formulae"][0]["versions"]["stable"])')" = "$version" ]; do
  [ "$(date +%s)" -lt "$deadline" ] || { echo "formula $formula not at $version after ${wait_seconds}s" >&2; exit 1; }
  sleep 30
done
brew upgrade "$formula" || brew install "$formula"
official="$(brew --prefix)/bin/innen"
[ "$("$official" --version | awk '{print $2}')" = "$version" ] || { echo "installed Homebrew innen is not $version" >&2; exit 1; }

dev="$bin_dir/innen"
if [ ! -e "$dev" ]; then echo "no dev build at $dev; nothing to retire"; exit 0; fi
if [ ! -f "$marker" ]; then
  echo "refusing: $dev has no dev-install marker (not installed by scripts/dev-install.sh); remove it by hand after checking" >&2; exit 1
fi
read -r m_commit m_dirty m_sha < <(python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print(d["commit"],str(d["dirty"]).lower(),d["sha256"])' "$marker")
[ "$(shasum -a 256 "$dev" | awk '{print $1}')" = "$m_sha" ] || { echo "refusing: $dev differs from the recorded dev install" >&2; exit 1; }
git -C "$repo" fetch --quiet --tags origin
git -C "$repo" merge-base --is-ancestor "$m_commit" "$tag" || { echo "refusing: dev commit $m_commit is not contained in $tag" >&2; exit 1; }
if [ "$m_dirty" != "false" ]; then
  # A dirty-tree build is retired only once that work is no longer pending:
  # the tree is clean now and everything committed since is part of the release.
  [ -z "$(git -C "$repo" status --porcelain --untracked-files=no)" ] || { echo "refusing: dev build came from a dirty tree and the tree still has uncommitted changes" >&2; exit 1; }
  git -C "$repo" merge-base --is-ancestor HEAD "$tag" || { echo "refusing: dev build came from a dirty tree and HEAD is not contained in $tag" >&2; exit 1; }
fi
rm -f "$dev" "$marker"
hash -r
resolved="$(command -v innen || true)"
echo "retired dev build $m_commit; innen -> ${resolved:-<none>} ($("$official" --version))"
[ "$resolved" = "$official" ] || { echo "warning: innen resolves to $resolved, not $official" >&2; exit 1; }

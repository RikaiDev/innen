#!/usr/bin/env bash
# Install the working-tree build as ~/.local/bin/innen with a provenance marker.
# This is the only sanctioned dev install: scripts/post-release.sh retires it once
# an official release containing its commit is installed through Homebrew.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${INNEN_DEV_BIN_DIR:-$HOME/.local/bin}"
marker_dir="${INNEN_DEV_MARKER_DIR:-$HOME/.local/share/innen}"
build=(cargo build --release)
if [ -x "$HOME/.agents/skills/machine-qos/scripts/cargo-singleflight.sh" ]; then
  build=("$HOME/.agents/skills/machine-qos/scripts/cargo-singleflight.sh" "${build[@]}")
fi
(cd "$repo" && "${build[@]}")
commit="$(git -C "$repo" rev-parse HEAD)"
dirty=false
if [ -n "$(git -C "$repo" status --porcelain --untracked-files=no)" ]; then dirty=true; fi
version="$("$repo/target/release/innen" --version | awk '{print $2}')"
mkdir -p "$bin_dir" "$marker_dir"
install -m 0755 "$repo/target/release/innen" "$bin_dir/innen.partial"
mv "$bin_dir/innen.partial" "$bin_dir/innen"
sha="$(shasum -a 256 "$bin_dir/innen" | awk '{print $1}')"
cat > "$marker_dir/dev-install.json" <<JSON
{"schema":"innen.dev-install.v1","path":"$bin_dir/innen","version":"$version","commit":"$commit","dirty":$dirty,"sha256":"$sha","installed_utc":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
JSON
suffix=""; [ "$dirty" = true ] && suffix=", dirty tree"
echo "installed $bin_dir/innen $version ($commit$suffix)"

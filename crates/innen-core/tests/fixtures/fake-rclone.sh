#!/usr/bin/env bash
# fake-rclone.sh — stub rclone for innen Task 11b tests. No network.
#
# Supports: lsd / rcat / cat / version / lsjson / delete|deletefile.
# Unknown argv → exit 3.
# rcat/delete append argv+byte counts to $FAKE_RCLONE_LOG (missing → /dev/null).
set -u

cmd="${1:-}"
case "$cmd" in
  lsd)
    # `lsd <remote>:` → canned dir listing (must contain "canned-dir").
    echo "canned-dir"
    ;;
  rcat)
    # `rcat <remote>/<file>`; stdin bytes counted, logged.
    dest="${2:-}"
    nbytes="$(cat | wc -c | tr -d ' ')"
    log="${FAKE_RCLONE_LOG:-/dev/null}"
    echo "rcat $dest bytes=$nbytes" >> "$log"
    ;;
  cat)
    # `cat <remote-path>` → canned bytes (non-empty, deterministic).
    printf 'canned-bytes-12345'
    ;;
  version)
    echo "rclone fake v1.0 canned-version"
    ;;
  lsjson)
    # `lsjson <remote>:` → canned minimal JSON array for prune.
    echo '[{"Name":"orphan-a.bin","IsDir":false},{"Name":"orphan-b.bin","IsDir":false}]'
    ;;
  delete|deletefile)
    dest="${2:-}"
    log="${FAKE_RCLONE_LOG:-/dev/null}"
    echo "delete $dest" >> "$log"
    ;;
  *)
    echo "fake-rclone: unknown command: $cmd" >&2
    exit 3
    ;;
esac

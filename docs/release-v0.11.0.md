# innen v0.11.0

Four defects in which a tool reported success while doing nothing, and two
surfaces an agent could not guess. All four defects shared one structure:
**treating a position or a substring as an identity.**

## Fixed

### The harvest tap could not see its own inbox

`harvest-dir` stored how many files it had consumed and selected with
`files.into_iter().skip(count)`, while each pending snapshot's own
instructions say to delete it after ingest. Every deletion shifted the window
by one, so once the count passed the listing size the tap reported an empty
backlog while unprocessed files sat in the inbox — silently, with exit 0.

On the reference machine that was a count of 875 against 96 files since
2026-09-20: **0 of 96 had ever been ingested**, and `harvest --check` kept
answering "no new files". `store_watermark` refused to decrease, so the count
could not be walked back.

The cursor now records which inputs were consumed
(`innen.tap.watermark.v2`). It survives a shrinking inbox and a file returning
to it, and storing it is no longer monotonic because consuming fewer files is
normal once files are deleted. A cursor that cannot name its inputs — the
pre-v2 bare count, or an unreadable file — re-lists the inbox instead of
guessing a count, which is safe because the journal event id is derived from
content and the op is an upsert. `harvest --check` reports that state as
`cursor`, and `doctor` gained a `tap_cursor` check that fails, rather than
letting it read as an empty backlog.

`check()` and `ingest::run()` now select through one shared function, so a dry
run cannot disagree with the run it previews.

### OpenCode sessions were invisible after 2026-09-09

OpenCode v2.0.22 renamed its tables (`session` → `session_v2`, `message` →
`session_message`, parts moved inline into `session_message.data` with its own
`type` column and `seq` ordering). innen hardcoded the v1 names, so
`innen conversation <current-session>` answered "not found" for a session with
1,711 messages, and `unfinished --source opencode` always returned nothing.

One schema decision now owns the table names, and both schemas emit the same
`{id, message, parts}` envelope so the projection layer keeps a single dialect.

### `wiki sync` stopped the whole graph over one page

`has_openai_key` matched the `sk-` substring anywhere, and its tail allows
`-`, so any kebab-case identifier ending in `task-` read as a key —
`task-resume-brief-structural-2026-09-06.md` is `sk-` plus a 34-character
tail. `wiki sync` aborts when any page trips the credential scan, so a page
whose only credential-like content was a file path silently kept itself and
every other page out of the graph.

`sk-` must now start a token; real keys are quoted, assigned, or at the start
of a token. Separately, an unreadable page is skipped and named instead of
aborting the projection. A skipped page keeps its node and its edges — a parse
error is not evidence that provenance went away — and the CLI exits non-zero so
a partial projection never reports success. Genuine ambiguity (two ids for one
path, one id claimed by two pages) stays fatal, because skipping would silently
pick a winner.

### grep-style invocations were rejected

An agent writing `innen query <words>` got `unexpected argument`, and the only
surface it could see was a 22-row command table with no intent in it. `query`
and `trace` now take bare terms joined onto `--q`, so grep syntax and the flag
form resolve to the same query and neither form drops a caller's words.

## Changed

| | Before | After |
| --- | --- | --- |
| `harvest --check` tap | `{id, new_files, skipped}` | adds `cursor` |
| `harvest-dir.watermark` | bare integer | `innen.tap.watermark.v2` |
| `wiki sync` report | `{pages_scanned, ...}` | adds `pages_skipped` |
| `wiki sync` exit | 0 or 2 | 0, 1 (partial), or 2 (fatal) |
| `doctor` checks | 4 | adds `tap_cursor` |
| top-level `--help` | command table first | intent index first |

A pre-v2 watermark is rebuilt on the next `innen ingest`, which re-lists the
inbox; replay is convergent, so files ingested before the upgrade are not
duplicated in the graph.

## Measured

| | Before | After |
| --- | --- | --- |
| inbox files never ingested | 96 of 96 | 0 of 105 |
| `harvest --check` reported | `new_files: []` | 105 files, `legacy_count_rebuilt` |
| `unfinished --source opencode` (7d) | no candidates | 70 candidates |
| `wiki sync` | aborts on page 1 | 105 pages, 41 edges asserted |
| `doctor` on the reference KB | cursor fault invisible | 5 checks, exit 0 |

168 tests pass; strict Clippy clean.
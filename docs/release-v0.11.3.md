# innen v0.11.3

The three defects the v0.11.2 splits turned up, fixed. One of them returned a
quote that did not contain the term the caller had searched for.

## Fixed

### `trace` excerpts could omit the term they were located by

`excerpt` searched `text.to_lowercase()` for the query and then applied the
resulting offset to `text`. Those are two different strings: lowercasing is not
byte-length preserving. `İ` is two bytes and lowercases to three, so every such
character before the match drifts the offset by one byte.

Below the excerpt's 120-byte lookback the drift is absorbed and nothing looks
wrong. Above it the window opens *after* the match, so the quote comes back
without the term it was found by — for a caller reading `quote` to decide where
to look, that is worse than no quote at all.

```
130 × "İ" + "NEEDLE" + 600 × "z", searching for "needle"
before: quote begins "…zzzzzzz…"   ← the term is gone
after:  quote begins "…İİİİ…"
```

The search no longer builds a lowercased copy. It compares case-insensitively
against `text` itself, so every offset is already in the right string's bytes.
For the ASCII case — everything the existing tests covered — the behaviour is
unchanged.

### A duplicated early return in session retention

`session_retention/policy.rs` carried the 21-line "bundle already removed" block
twice, byte for byte including its comment. The first copy returns
unconditionally, so the second was unreachable. It was a copy-paste artefact of
the v0.10.2 `already_cleaned` fix.

### `task_entry::CandidateItem` was never constructible

The struct was defined and re-exported and appeared nowhere else in the
workspace — nothing built one. Removed, along with the `serde` imports that only
it needed.

## Changed

| | before | after |
| --- | --- | --- |
| `graph` materialization of `edge.assert` / `edge.retract` | `typ.parse().unwrap()` × 3 | `typ.parse().unwrap_or_else(\|never\| match never {})` |

`EdgeType::Err` is `Infallible` — unknown names become `Custom` — so this is not
a behaviour change. It removes a panic path that only the type signature was
preventing, and the `match never {}` form fails to compile rather than panicking
if the error type ever gains a variant.

## Measured

| | before | after |
| --- | --- | --- |
| `excerpt` regression test on the failing input | quote missing the term | quote contains the term |
| unreachable lines in `session_retention/policy.rs` | 21 | 0 |
| unused `pub` types in `task_entry` | 1 | 0 |
| panic-capable `unwrap()` in graph materialization | 3 | 0 |

407 tests pass, 0 failed. `cargo clippy --workspace --all-targets
--all-features --locked -- -D warnings` clean. `cargo fmt --all --check` clean.

## Note on how the regression test was found

The first two versions of that test passed against the broken code — a single
`İ` does not produce enough drift, and without trailing content the
`at.min(text.len())` clamp hides it. A test that passes while the defect is live
is the same failure this project has hit repeatedly: it asserts one shape of
input and reads the result as a contract. The third version was confirmed
failing on the unfixed code with a standalone probe before it was committed.
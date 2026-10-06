# innen v0.10.2

A session whose native bundle is already gone is reported as cleaned, not as an
error.

## Fixed

A store can keep a session row after its files are deleted. `assess_candidate`
then ran every gate and finally failed inside `hash_targets` with "session
bundle has no existing targets", which surfaced as `assessment_error`.

That made `plan` and `sweep` fail forever on rows with nothing left to clean.
On the reference machine it had reached 164 of 554 sessions, so every report
carried 164 phantom errors and the inventory overstated the real backlog by 30%.

The check now runs first, before any gate: an absent bundle has no bytes to
protect, no attestation to prove, and nothing to purge, so `already_cleaned` is
the honest verdict. Genuine failures keep `assessment_error` — a bundle that
exists but cannot be read is still reported, and a test pins that distinction.

## Measured

| | Before | After |
| --- | --- | --- |
| `assessment_error` | 164 | 0 |
| Inventory total | 554 | 382 |

A follow-up sweep removed a further 19 sessions and freed 100.8 MiB. The
remaining 353 sessions are opencode ones reporting "native session has an open
file handle" while the tool is running: that gate is correct, since a live
session's bytes are not deletable.

## Note on the rebase

The fix was rebased onto a concurrent merge that also touched
`session_retention/policy.rs`. The rebase took the other side of that file and
dropped the decision while keeping the constant and its tests, and the suite
stayed green because the tests assert the blocker code, which was still
declared. The check is re-applied at the top of `assess_candidate` in its own
commit.

# Decision history in context middleware

Decision events remain outside the model's working context. The history selector
projects current decisions, necessary causal dependencies, and correction lessons
for one exact project and action. It performs no model inference or keyword search.

## Input

Use a JSON envelope with `stable_prefix`, `task`, `budget_tokens`, and chronological
`events`. `task` contains stable `id`, `project`, `action`, `state`
(`active`, `paused`, or `completed`), and observed `conditions` (string map).
Each immutable event contains:

| Field | Contract |
|---|---|
| `id` | Unique nonblank decision identity |
| `task_id` | Persistent task identity, independent of session and calendar date |
| `project` / `actions` | Exact project identity and action names |
| `observed_at` | Canonical UTC `YYYY-MM-DDTHH:MM:SSZ` |
| `statement` | Decision text at that point in history |
| `rationale` | Why that decision was made |
| `conditions` | Facts that must match the task for the decision to apply |
| `sources` | Source IDs and SHA-256 versions |
| `supersedes` | Earlier decisions explicitly replaced by this event |
| `depends_on` | Earlier same-project decisions required to use this decision |
| `lessons` | Relevant failures or counterexamples to retain |
| `reopen_when` | Evidence or circumstances warranting reconsideration |

The caller supplies reviewed facts and source versions. Hash syntax validation
does not establish source existence or semantic truth. Keep those source bytes
and their verification receipts with the owning project. New corrections append
new IDs; do not overwrite old events. The full envelope hash changes when any
event or task input changes.

Each source may include `session_id` as provenance. Sessions do not partition
the decision history, and this selector applies no yesterday/today cutoff.
Save the history envelope under the owning task outside transient chat context.
Any new session can read that same file and reconstruct the packet for its task
ID. Switching to an urgent task selects a different task ID. Paused/completed
tasks cannot emit an active dispatch packet; resuming explicitly sets the task
active and supplies fresh condition observations. Stored source hashes alone
do not establish live source freshness.

## Persistent private store

For durable local history, save a complete single-task envelope to a private
store and later load it by its exact project and task identity:

```bash
innen middleware history-save --file history.json --store /absolute/private/history-store
innen middleware history-load --store /absolute/private/history-store \
  --project project:tool --task task:long
```

The store hashes `project + NUL + task` for its directory and retains immutable
snapshot bytes under the validated full-envelope SHA-256. A locked append-only
per-task index selects the latest complete envelope. Every stored event must
belong to that exact project and task. A later save may explicitly change the
task action, state, or conditions, but cannot remove or rewrite any earlier
decision ID for that task. Corrections therefore add a new decision that
supersedes the old one. Corrupt, incomplete, or mismatched index/snapshot data
fails closed; it is never silently repaired or partially loaded. Store
directories use owner-only Unix permissions where supported.

Task metadata is an explicit caller snapshot: concurrent valid state/condition
updates serialize under the store lock and the last committed snapshot is the
one `history-load` returns. The store does not infer semantic conflicts or
refresh conditions automatically.

Saving and loading validate the full decision structure even for paused and
completed tasks. Preparation remains limited to active tasks, so a loaded
paused/completed envelope can be inspected or retained but cannot dispatch a
packet until it is explicitly resumed.

## Selection and expansion

```bash
innen middleware history-prepare --file history.json
innen middleware history-prepare --file history.json --packet-only
innen middleware history-expand --file history.json \
  --expect-source-sha256 HISTORY_HASH --ids previous-decision
```

Both prepare and expand accept stdin with `--file -`. Preparation includes the
current decisions for the exact project/task/action, their necessary dependencies and
correction ancestors. Superseded statements are available through expansion;
their rationales, lessons, reopening conditions and source references remain in
the causal history summary. Unrelated tasks are excluded. Cross-project or cross-task
references are rejected; explicitly shared rules belong in the stable prefix.

Conditions missing or mismatched withhold the affected active statement and mark
the decision guarded. Competing supersession branches expose a conflict. A stale
dependency is guarded rather than silently reviving a replaced statement. These
results return exit 2; packet-only stdout remains empty, preventing accidental
dispatch. Normal receipts retain diagnostics for inspection. Unmatched tasks,
malformed references and insufficient budgets also fail explicitly.

All selected causal content is required in the generic middleware budget. If it
does not fit, narrow the task or deliberately increase the budget. Selection does
not silently drop the correction lesson to improve a token score. Input is limited
to 16 MiB and 256 decisions; timestamps and reference order prevent cycles.

## Native Codex adapter

The following command consumes a native hook event on stdin:

```bash
innen middleware history-hook --file /absolute/private/history.json \
  --cwd /absolute/owning/workspace
```

It can instead load one explicitly bound task from the persistent store:

```bash
innen middleware history-hook --store /absolute/private/history-store \
  --project project:tool --task task:long --cwd /absolute/owning/workspace
```

`--file` and `--store` are mutually exclusive. Store mode still requires the
same canonical workspace and native session checks; it does not infer a task
from the hook event.

It handles `SessionStart` with `startup`, `resume`, `clear`, or `compact`. An exact
canonical workspace match and a session identity are required. It supplies the
prepared context through `hookSpecificOutput.additionalContext`; mismatch emits
an empty result. Unavailable history emits a diagnostic and does not stop Codex.
Guarded decisions remain explicitly guarded in the hook's diagnostic context.

Configure this command only for a reviewed, active task history file. The adapter
does not discover the user's new task from a startup event, decide that a task is
still active, or remove native context. Native hook registration/trust and actual
session consumption are separate from CLI testing. This release does not install
or activate global hooks automatically.

## Evaluation

Regression cases cover cleanup while another consumer is active, correction of
an overbroad documentation decision, and exclusion of private operational records
from public releases. Fixtures are synthetic; no private conversation is published.
Checks assert that current decisions and correction lessons survive, obsolete
statements are absent from active packets, unrelated scopes are excluded, exact
expansion works and guarded states cannot silently dispatch.

Token comparisons use the same task's full history versus its causal projection.
They measure reference input size, not model decision accuracy, billing or total
subscription usage. Native integration requires an additional observed session
test before claiming automatic context management.

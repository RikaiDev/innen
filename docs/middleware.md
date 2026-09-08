# Context middleware

For automatic selection from structured decisions and their correction history,
see [decision history](history-context.md). The generic interface below remains
available for explicit caller-selected source items.

`innen middleware` prepares bounded context before an agent executor assembles a
model request. It runs offline and does not start a model, change a parent model,
or replace a running Codex session's internal history.

## Executor contract

The executor supplies a stable instruction prefix, current task, and ordered
source items. Mark constraints, authorization, active failures, counterevidence,
and anything necessary for acceptance as `required: true`. Only the caller can
decide relevance; the middleware never infers that an omitted item is irrelevant.

```json
{
  "stable_prefix": {"instructions": "Use the supplied evidence; expand when needed."},
  "task": {"goal": "Inspect the latest build state"},
  "items": [
    {"id": "constraints", "content": "Do not publish before tests pass", "required": true},
    {"id": "previous-poll", "content": "Build running", "required": false, "priority": 0},
    {"id": "latest-poll", "content": "Build failed; inspect diagnostic receipt", "required": true}
  ],
  "budget_tokens": 2048
}
```

Prepare an audit receipt and the actual input packet from the same frozen file:

```bash
innen middleware prepare --file request.json
innen middleware prepare --file request.json --packet-only
```

`--file -` accepts stdin. The executor must check exit status before forwarding
stdout. `--packet-only` writes nothing to stdout on failure. A budget too small
for required context returns exit 2 rather than truncating evidence. The budget
covers serialized packet reference tokens, not the executor's other tools,
instructions, history, or provider billing.

Keep the full request outside the model context. Resolve omitted source items
only against the receipt's source hash:

```bash
innen middleware expand --file request.json \
  --expect-source-sha256 HASH --ids previous-poll
```

Changed sources require a new preparation. The selected packet preserves input
item order; optional selection follows explicit priority and then input order.
It is a bounded selection policy, not a proof of globally optimal compression.
Input is limited to 16 MiB and 256 items. Packet-only JSON serializes the stable
prefix first, task second, and selected items last; reported packet token counts
cover that exact serialization. This preserves a reusable leading section, but
actual cache hits remain a property of the executor's full provider request.

## Where to integrate

- Tool-result boundary: retain full logs locally; supply caller-selected result
  objects and exact expansion sources to the middleware.
- Task boundary: construct a fresh request from required decisions and evidence,
  then let the executor use its supported new-session or compaction interface.
- Request boundary: forward only the packet after successful preparation. Sending
  a shorter packet alongside the entire old conversation does not remove history.

A native session that exposes no supported request-replacement interface cannot
be transparently shortened by a CLI invocation. This command is an integration
boundary for executors, not an installed interception proxy.

## Waiting and measurement

An external monitor should poll CI/build state and wake the agent for a meaningful
transition or completion. Repeatedly asking a model to decide to poll again still
reloads its context even when each tool result is short.

Evaluate completed tasks with the same acceptance criteria. Include extraction,
cache misses, expansion, retries, output, and all executor calls. Reference-token
reduction on the supplied packet is measurable offline; runtime savings require
observed request usage. Subscription percentages are separate account telemetry.

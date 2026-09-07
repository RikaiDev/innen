# innen (因縁) — a deterministic, CLI-first realization of the LLM Wiki

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 2021](https://img.shields.io/badge/Rust-2021%20%28MSRV%201.90%29-orange.svg)](https://www.rust-lang.org/)
[![Architecture: Event--Sourced](https://img.shields.io/badge/Architecture-Event--Sourced%20Graph-green.svg)](#architecture)
[![Zero-Daemon](https://img.shields.io/badge/Runtime-Zero--Daemon%20CLI%20Binary-black.svg)](#why-zero-daemon-cli-over-mcp)
[![Read: Local Sessions](https://img.shields.io/badge/Read-Local%20Sessions-blue.svg)](#read-a-conversation-directly-by-session-id)

**innen is an open-source, high-performance knowledge engine and graph runtime designed for LLM coding agents. Built as a single, zero-daemon Rust binary, it implements an append-only event-sourced journal, dual embedded derived indexes (Redb + Tantivy CJK BM25), and read-only coding-tool conversation adapters. Source retrieval and durable knowledge ingestion are separate operations.**

*The name reads from Buddhist philosophy: **因縁** (innen) — dependent origination, causal connection. In genuine knowledge management, no claim exists in a vacuum. Every decision, task, experiment, and code artifact arises out of prior conditions and leaves traceable consequences. innen does not merely store text; it captures the causal graph of why things are the way they are.*

---

## Start with the question

```bash
innen project                         # What remains across recorded projects?
innen project nhri-pcne                # What remains in this project?
innen project nhri-pcne --view evidence # Why? Expand task status history and source events.
innen resume <session-id>              # Read conversation context when evidence requires it.
innen query --q '<term>'               # Find prior knowledge when the project is unknown.
```

These are user questions, not a mandatory command sequence. No Python or direct
journal/transcript parsing is needed to answer the project task question.
`checkpoint`, `harvest`, and index maintenance remain supporting operations;
`unfinished` finds heuristic conversation candidates, not confirmed business tasks.
No new top-level task command is introduced.

`project` now defaults to compact recorded task rows. It replays status updates,
excludes only explicit terminal statuses, deduplicates memberships, respects current
edge validity and retractions, and retains unknown status and unassigned tasks.
The project dictionary also includes projects with no recorded pending tasks;
that absence does not prove all work is complete. Recorded next actions and
blockers are included without inventing them from conversation snippets.
Use `--limit` and `--offset` with `next_offset` for larger portfolios. Paging reads
current state, so repeat the query if another writer changes the journal mid-read.

**Output migration:** consumers of the previous `{id,render}` project response
must use `project <id> --view full`. That view preserves the legacy page contract,
including its unfiltered Tasks section. `--view evidence` includes terminal tasks,
merged node metadata and append-only history with event IDs and source line numbers.
The default brief is a navigation layer, not lossless compression of that history.

`query` is also the task-context entry when the project is not yet known. Its
default `--view context` accepts the original natural-language request and
returns bounded candidate rows, recorded project links, decisions, authority
gaps, and a structured evidence-expansion argv. Use `--view hits` for the
legacy lexical-plus-graph hit wire shape, or `--view evidence` to expand the
selected rows with node payloads and append-only journal history. Query scope
is deterministic: `--root` wins, then an absolute `INNEN_ROOT`, then the
absolute root registered by `innen config set --global root <path>`; an
unconfigured or malformed global root fails explicitly and never creates a
`.innen` directory in the current working directory. `--as-of`, `--offset`,
`--limit`, and `--include-expired` are preserved in expansion handles.

The context view reports lexical candidate resolution and provenance, not
semantic certainty. A filename or `final` status alone is not an approved
editable baseline; missing or conflicting authority remains `missing` or
`ambiguous`.

### v0.2.0 migration

Register the knowledge-base root once with an absolute path:

```bash
innen config set --global root /absolute/path/to/knowledge-base
```

An explicit `--root` takes precedence, followed by a non-empty `INNEN_ROOT`;
the registered global root is used otherwise. The root must be absolute.

The default `query` view is now `context`; use `--view hits` for the legacy
lexical-plus-graph hit shape and `--view evidence` for expanded evidence. The
default `project` output is the compact brief; use `project <id> --view full`
for the legacy full page. `resume <session-id>` emits the bounded brief by
default; use `--view context`, `--view dialogue`, or `--view events` when a
different page is needed.

Verify which executable is being used when Homebrew and a local wrapper are
both installed:

```bash
command -v innen
innen --version
```

`~/.local/bin/innen` or another earlier PATH entry can shadow the Homebrew
installation. Compare the reported path and version before troubleshooting.

The repository also contains local research harnesses for evaluating output
length and retrieval fidelity. They are excluded from the release package and
their private corpora are not release evidence; output size is not billed-token
savings or a comprehension guarantee.

## 1. Context & Motivation

In April 2026, **Andrej Karpathy** published his formal [LLM Wiki Guide](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f), outlining a radical shift away from opaque corporate AI memory silos toward an open, explicit, compounding knowledge base:

> *"In the age of LLM agents, sharing specific code or apps is no longer necessary. You only need to share the core idea (an idea file), and everyone's personal agent will tailor-build the realization to their exact needs."*

Karpathy articulated four foundational pillars of the **LLM Wiki**:
- **Explicit**: The knowledge is laid out in plain sight; you can inspect exactly what the AI knows and what it does not.
- **Yours**: Held on your own machine under your direct ownership, free from vendor lock-in.
- **File over app**: Stored in durable formats (Markdown, JSON, images) readable by standard Unix tools.
- **BYOAI (Bring Your Own AI)**: Model-agnostic; agents across different providers can all read and write to it.

### The Limits of Naive Markdown at Scale

Karpathy's initial observation was elegant: *"At small scale (~100 articles, ~400k words), no RAG is needed; the LLM simply navigates through index files."*

However, once an agentic knowledge base scales to hundreds of daily transcripts across multiple coding tools, multi-project experiment matrixes, and evolving research state, naive flat markdown approaches hit structural ceilings:

1. **Context Window & Token Tax**: Dumping raw transcripts and verbose ledgers into prompts rapidly inflates token costs (2,500 – 5,000+ tokens per retrieval), slows inference, and causes context dilution / needle-in-a-haystack forgetting.
2. **Ephemeral Chat Evaporation**: Modern developers switch between multiple coding tools (Codex, Claude Code, Gemini/Antigravity, OpenCode, Grok). Valuable decisions, root-cause analyses, and test receipts vanish into gigabytes of scattered JSON/JSONL logs.
3. **Loss of Temporal Provenance**: Flat file edits overwrite past state. When was a decision made? Which experiment invalidated it? Overwrites destroy causal continuity.
4. **Daemon Bloat & Vector Flaws**: Traditional memory engines rely on background daemon processes, open HTTP ports, and non-deterministic vector databases that miss exact hashes and issue IDs while consuming excessive battery and memory.

**`innen` is an opinionated, production-grade realization of the LLM Wiki paradigm.** It bridges human-readable markdown with the mathematical rigor of an append-only event journal, deterministic graph retrieval, and multi-agent conversation harvesting.

---

## 2. Why Zero-Daemon CLI over MCP?

Rather than implementing a long-running Model Context Protocol (MCP) server daemon, **`innen` is engineered strictly as a zero-daemon, high-performance CLI (similar to `rtk`, `rg`, and `git`)**.

| Dimension | MCP Server Daemon | innen CLI (`rtk`-style) |
|---|---|---|
| **Context Window Overhead** | **1,500 – 2,500 tokens / turn** (Mandatory tool schemas in system prompt) | **0 tokens** (Zero schema tax; tool definitions loaded on demand via skills/AGENTS.md) |
| **Agent Ecosystem Reach** | Fragmented (Requires separate client configs for Claude Desktop, IDE extensions, etc.) | **100% Universal** (Invoked via standard `run_command` / bash across Codex, Claude, Gemini, OpenCode, Grok) |
| **Runtime Footprint** | Persistent background daemon, memory leakage risk, open ports | **Stateless**: 17ms cold start, <7MB RAM, terminates immediately after execution |
| **Concurrency & Locks** | Daemon contention over SQLite / Redb WAL locks | Clean file locks; zero long-lived handles |
| **Human Ergonomics** | Opaque RPC protocol inaccessible from terminal | First-class Unix citizen (`innen query --q "..." \| fzf`, `innen project <id> \| pbcopy`, git hooks) |

These CLI/MCP comparisons describe packaging tradeoffs, not a paired benchmark of total agent token usage. Discovery, instructions, command arguments, output, and cache behavior still have costs. The OpenCode source reader uses the local `sqlite3` executable.

---

## 3. Conversation Operations and Knowledge Boundaries

`innen` strictly separates four conversation-related operations:

1. **Harvest (`innen harvest` / `innen ingest`)**: Imports conversation transcripts into durable knowledge graph entities and journal events. `harvest --check` inspects staged Markdown in `00-inbox/harvest`; `ingest` appends them to `.innen/journal.jsonl`.
2. **Checkpoint (`innen checkpoint`)**: Records progress snapshots (`active`, `blocked`, `completed`, `superseded`) in append-only local storage outside Git (`.innen/checkpoints.jsonl` with `.innen/.gitignore`). Checkpoints record goals, completed work, observations, blockers, next actions, and verification without modifying Git or rewriting prior history.
3. **Unfinished Discovery (`innen unfinished`)**: Discovers recent candidate conversations across local agent stores (`agy`, `codex`, `claude`, etc.) using structural evidence (unanswered user requests, interrupted turns, unresolved tool failures, recent activity). Candidates are explicitly labeled `candidate` based on structural heuristics, never fabricated semantic certainty. Completed or superseded sessions are excluded.
4. **Context Continuation (`innen resume` / `innen pickup`)**: Reconstructs working context without full transcript dumps:
   - `resume`: Emits structured context (`--view context --compact --deltas`) with line provenance.
   - `pickup`: Designed for a fresh agent session. Selects only when unambiguous (or returns concise candidate choices), emits compact checkpoint + evidence pointers + exact resume command, and never executes transcript text.

`read <session-id>` retrieves local source conversations directly. It does not
create graph nodes or infer decisions. `harvest --check` currently inspects
Markdown already present in `00-inbox/harvest`; `ingest` processes that inbox.
A successful source read or pickup must not be reported as completed knowledge ingestion.

The dialogue view includes user and assistant text. Tool results, control events,
and original source fields can be inspected with `--view events` when checking
claims. Filtering text is not evidence that the remaining text preserves every
fact or supports a better answer.

Historical local benchmark claims are omitted from this release. The public
contract documents retrieval behavior and provenance; local output-size studies
require private corpora and excluded research tooling, and do not establish
answer accuracy, knowledge completeness, or billed-token savings.

---

## 4. Architecture

`innen` strictly decouples immutable ground truth from throwaway, high-speed derived indexes:

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                            The LLM Agent                                │
│          (Codex / Claude Code / Gemini / OpenCode / Grok)               │
└───────────────────▲─────────────────────────────────┬───────────────────┘
                    │ query / project (<35ms)         │ harvest / ingest / append
                    │ (90% – 95% token reduction)     │ (append-only)
┌───────────────────┴─────────────────────────────────▼───────────────────┐
│                               innen CLI                                 │
│                      (Single Static Rust Binary)                        │
├────────────────────────────────┬────────────────────────────────────────┤
│  Derived Indexes (Rebuild 30ms)│        Source of Truth (Append-Only)   │
│  ┌──────────────────────────┐  │  ┌──────────────────────────────────┐  │
│  │   Tantivy CJK FTS        │  │  │   .innen/journal.jsonl           │  │
│  │   (BM25 + Jieba CJK)     │  │  │   (Canonical JSON, SHA256 ID,    │  │
│  ├──────────────────────────┤  │  │    Bi-temporal: observed/valid)  │  │
│  │   redb (Embedded KV)     │  │  └──────────────────────────────────┘  │
│  │   (ACID, Adjacency Graph)│  │  ┌──────────────────────────────────┐  │
│  └──────────────────────────┘  │  │   01-raw/ & 02-wiki/ Markdown    │  │
│                                │  └──────────────────────────────────┘  │
└────────────────────────────────┴────────────────────────────────────────┘
```

### Core Principles

- **Append-Only Event Sourcing**:
  Every entity (`node.upsert`), relation (`edge.assert`), and cancellation (`edge.retract`) is recorded as an immutable JSON line in `.innen/journal.jsonl`. Edits never erase history; retraction preserves historical auditability while updating materialized graph views.
- **Bi-Temporal Modeling**:
  Events record both `observed_utc` (when the system recorded the event) and `valid_from` / `valid_until` (when the fact was true in the real world). Queries traverse historical snapshots using `--as-of <TIMESTAMP>`.
- **Zero-Daemon Dual Indexes**:
  - **Redb**: Embedded, pure-Rust transactional key-value store maintaining materialized node tables and adjacency graphs.
  - **Tantivy CJK FTS**: Embedded full-text search engine with bundled Jieba CJK segmentation. No background services, no open ports, zero cloud dependencies.
- **Instant Index Rebuilds**:
  Indexes are strictly derived state. Corrupted or stale indexes can be regenerated from scratch via `innen index rebuild` in **under 35 milliseconds**.
- **Deterministic Integrity (`doctor` / `lint`)**:
  Zero-mutation health checks ensure all edge endpoints resolve, quarantine directories remain empty, and journals parse flawlessly, returning strict exit codes (`0` for clean, `1` for quarantined, `2` for corruption/dangling edges).

---

## 5. Empirical Retrieval Benchmarks

Tested on a production research knowledge base comprising **635 events, 395 relational edges, 85 structured wiki pages, and 11 active project ledgers** on Apple Silicon:

### A. Historical Execution Latency & Memory Footprint

| Command | Description | Mean Latency | Peak Memory (RSS) |
|---|---|---|---|
| `innen --help` | CLI cold startup & argument parsing | **17.3 ms** | **2.9 MB** |
| `innen lint` | Full journal & reference integrity audit | **33.4 ms** | **5.5 MB** |
| `innen status` | Materialized graph census & node breakdown | **29.4 ms** | **6.6 MB** |
| `innen project <id>` | Deterministic project page synthesis | **21.3 ms** | **6.4 MB** |
| `innen index rebuild` | Full Redb + Tantivy index regeneration | **30.1 ms** | **2.8 MB** |
| `innen query --q <term>` | Jieba tokenize + BM25 + 3-hop BFS expansion | **309.7 ms** | **76.4 MB** (includes CJK dict) |

The table itself reports `query` at 309.7 ms and 76.4 MB. The earlier blanket claim that all inspection commands take under 35 ms and under 7 MB was incorrect. These historical measurements have not been rerun for the new reader.

### B. Historical Output-Length Comparison

When an agent needs context on a specific project or topic, retrieval strategies yield vastly different token loads:

```text
[Context Window Injection Comparison]
────────────────────────────────────────────────────────────────────────────────────
1. Naive Flat Dump (Raw Transcripts + Full Wiki)  ████████████████████ 2,500 tokens (100.0% Baseline)
2. Verbose Project Ledger Dump                   ██████████████████   2,252 tokens (90.1%)
3. Unranked Graph Entity List                    ██████               739 tokens (29.6%)
4. innen query --q <term> (BM25 + 3-Hop BFS)     ██                   245 tokens (9.8%)  🔥 90.2% reduction
5. innen project <id> (Structured Synthesis)     ▌                    102 tokens (4.1%)  🔥 95.9% reduction
────────────────────────────────────────────────────────────────────────────────────
```

These are output-length comparisons against the stated baseline. They do not
show that the commands returned equivalent information or that a model answered
better. The previous claims of an "exact mental model" and "100% causal recall"
were unsupported by a reported comprehension test.

---

## 6. Quick Start

### Installation

If the v0.2.0 formula has been published to the configured tap, install it via Homebrew on macOS:

```bash
brew install rikaidev/tap/innen
```

Or build from source (the OpenCode reader additionally uses `sqlite3` on PATH):

```bash
git clone https://github.com/RikaiDev/innen.git
cd innen

# Run the test suite
cargo test --workspace

# Build optimized release binary
cargo build --release

# Install to PATH
cp target/release/innen /usr/local/bin/innen
```

### Read a conversation directly by session ID

```bash
innen read e3a92b35-4931-427b-9adf-1baa29318ca6
innen --format json read <session-id> --view context --compact --deltas --limit 100
innen --format json read <session-id> --lines 175,307 --compact --deltas
innen read <session-id> --view events --limit 20
innen --format json read <session-id> --view events --compact --limit 100
innen read <session-id> --offset <next_offset>
innen read ses_... --source opencode
innen read <session-id> --source claude --source-root /relocated/projects
```

`conversation` is an alias-equivalent command. Auto discovery matches full UUIDs
or native OpenCode `ses_...` IDs, never a fuzzy prefix. Use `--source` to select
one tool. `--source-root` requires `--source` and overrides its store location.
Multiple matching files are an explicit ambiguity error, not an arbitrary choice.

### Resume a conversation into working context

```bash
# Automatically discover and resume unambiguous session for current project
innen resume

# Resume explicit session with structured context view and compact+deltas encoding
innen resume <session-id>

# Resume with specific source, lines, or project override
innen resume --project /path/to/project --source codex
innen resume <session-id> --lines 175,307
innen resume <session-id> --offset 20 --limit 20
```

`resume` is a self-describing entry point designed for seamless session continuation across coding agents. Unlike raw `read` which defaults to unstructured dialogue, `resume` defaults to `--view context --compact --deltas`, factoring repeated payloads and preserving exact line provenance without inventing semantic summaries.

When called without a session ID (`innen resume`), it inspects available project/session metadata across local stores. If exactly one unambiguous candidate session matches the current project, it resumes it immediately; otherwise it returns concise candidate choices with IDs and explicit ambiguity, never silently choosing another project.

### Record, inspect, and list checkpoints

Checkpoints provide deterministic, local-only progress tracking stored outside Git in `.innen/checkpoints.jsonl` (guarded by an automatic `.innen/.gitignore`):

```bash
# Record an active checkpoint for a session
innen checkpoint record --session <session-id> --source codex \
  --status active \
  --objective "Implement search indexing" \
  --completed "Created schema,Wired Tantivy writer" \
  --evidence "Index file written: target/idx" \
  --next-action "Add CJK tokenizer tests" \
  --verification "cargo test --test search"

# Show the latest checkpoint for a session (or auto-discover project candidate)
innen checkpoint show <session-id>
innen checkpoint show

# List all latest session checkpoints in the project
innen checkpoint list
```

### Discover unfinished candidate conversations

```bash
# Discover unfinished candidate conversations from yesterday and today (default)
innen unfinished

# Discover unfinished conversations with custom cutoff (e.g. 3d, 48h, YYYY-MM-DD, RFC3339)
innen unfinished --since 3d

# Filter candidate discovery by source agent
innen unfinished --source agy

# Portfolio discovery across all projects (Codex, agy, claude, etc.)
innen unfinished --all-projects

# Portfolio discovery filtered by source with custom cutoff
innen unfinished --all-projects --source codex --since 48h
```

Candidates are classified by structural evidence (`interrupted_or_cancelled`, `trailing_user_request_unanswered`, `unresolved_tool_failure`, `recent_activity_uncheckpointed`) with confidence scores. Any session with an explicit `completed` or `superseded` checkpoint is automatically excluded.

When `--all-projects` is specified, `innen` scans across all projects in local agent stores without requiring a current project directory. Each candidate includes its `project` provenance (e.g. `/Users/username/Workspace/project`) or an explicit `"unknown"` identity if no workspace was recorded. Checkpoints remain strictly per-project: if a candidate has a known project directory with `.innen/checkpoints.jsonl`, its per-project checkpoint is respected (`completed`/`superseded` excluded; `active`/`blocked` attached). Project-scoped discovery (`innen unfinished` without `--all-projects` or with `--project <dir>`) remains strictly isolated to that project.

### Pick up an unfinished conversation for continuation

```bash
# Automatically pick up an unambiguous unfinished session for the current project
innen pickup

# Pick up an unambiguous unfinished session across all projects
innen pickup --all-projects

# Pick up a specific session by ID
innen pickup <session-id>
```

When unambiguous, `pickup` returns the target session, latest checkpoint, structural evidence pointers, and the exact `innen resume` command for continuation. It never executes transcript text. If multiple candidates match, it returns an explicit ambiguity list with IDs, project provenance, and reason codes, never guessing or silently picking another project.

| Source | Default store / format |
|---|---|
| `agy` / `antigravity` | `~/.gemini/{antigravity-cli,antigravity,antigravity-ide}/brain/<id>/.system_generated/logs/transcript.jsonl` |
| `codex` | `~/.codex/{sessions,archived_sessions}/**/*<id>.jsonl` |
| `claude` | `~/.claude/projects/**/<id>.jsonl` |
| `qwen` | `~/.qwen/projects/*/chats/<id>.jsonl` |
| `gemini` | `~/.gemini/tmp/**/session-*.json` or `.jsonl`; full metadata ID checked |
| `opencode` | `~/.local/share/opencode/opencode.db`; session/messages/parts queried read-only |
| `grok` | `~/.grok/sessions/**/<id>/chat_history.jsonl` |
| `copilot` | `~/.copilot/session-state/<id>/events.jsonl` |
| `cursor` | `~/.cursor/projects/**/<id>.jsonl` exported agent transcripts |
| `vscode` | Code / Code - Insiders / VSCodium `User/workspaceStorage/*/chatSessions/<id>.json` under macOS Application Support or Linux `.config` |

OpenCode requires `sqlite3` on PATH; other adapters do not launch another program.
Database-only Antigravity, Cursor's private database, cloud-only histories, and
unlisted source format versions are not decoded. Tool support here refers to the
listed source formats, not every past or future version. Missing data is reported
as unavailable, never as an empty successful conversation read.

Default `dialogue` projects source-order user and assistant text without semantic
summarization. `events` retains original JSON fields (OpenCode combines each
message with its stored parts; VS Code returns each request/response object).
Gemini JSONL revision/rewind controls remain visible in dialogue history; this is
not a reconstruction of the tool's current resume branch. VS Code dialogue
returns an ordered user/assistant exchange per request.

Pages include source path, one-based source record position (`line`), original
identifiers/timestamps where present, and `next_offset`. Offsets count zero-based
JSONL rows, document messages/requests, or ordered database messages, independently
of the view. `next_offset: null` means no further matching event was found.
The default limit is 20 matching records (maximum 100); event text is not newly
clipped, so page bytes depend on source event size. Original truncation markers
are retained. Cursor exports can omit tool outputs, so even events view may be
incomplete ([Cursor staff explanation](https://forum.cursor.com/t/accessing-the-full-agent-transcript-in-cursor/157311)).

An empty page beyond the end is different from a missing conversation. Invalid
IDs, missing or ambiguous sources, and corrupt JSON return distinct nonzero
errors. Sources may grow between pages; offsets assume append-only history.
Private source data is untrusted: the reader neither executes it nor writes it
into the journal. No token savings or comprehension advantage follows solely
from supporting these formats.

For understanding a Codex conversation, `--view context` keeps available user
and assistant text in source order beside explicitly partial tool previews.
Long previews retain the first and last 60 characters, with source positions,
character counts and truncation flags. Known exporter headers supply reported
exit codes; these do not establish task success. Explicit unique call IDs may
be represented by an in-page `call_line`; adjacency is never treated as proof.
Media-only messages retain nontext cues, and error, warning, aborted-turn and
compaction controls remain visible. Routine runtime metadata is omitted.
Other source dialects currently fall back to native events in this view.

Use `--view index` for navigation previews without full human text. Previews
cannot establish absence of a constraint or failure in the omitted middle.
Retrieve decisive evidence with `--lines 175,307`: up to 100 unique positive
source positions, returned as raw events in original order in one traversal.
This conflicts with pagination and attachment retrieval options. Follow each
context/index page's `next_offset` until null; source access remains necessary.
This is deterministic middleware with zero model calls, not automatic semantic
summarization or a claim that discarded metadata can never matter.

`--compact` is an opt-in, model-free middleware encoding of the selected page.
It factors repeated event fields and exact repeated long strings without
summarizing, reordering, or discarding occurrences. Use `--view events` when
tool evidence and all native fields are needed; compact dialogue retains only
the existing dialogue projection. Each page is self-contained.

`innen.rows.v1` supplies `layouts` with ordered `fields` and constant `shared`
fields. Its `rows` are `[source_line, layout_index, values]`. An optional outer
`innen.strings.v1` supplies `texts`, a collision-free `ref_key`, and `page`:
singleton objects using that key reference a zero-based string-table entry.
Expand string references first, then merge each row's values with its shared
fields. The Rust `conversation::compact::decode` API restores the ordinary page.
Unknown nested fields, time information, warnings and pagination survive this
round trip; JSON whitespace and source key order are not reconstructed.

Each transformation falls back when serialized JSON bytes would grow. This is
not a tokenizer guarantee: shorter bytes can still use more tokens. No model
calls, training or semantic inference are involved. The encoding's comprehension
cost and tokenizer savings must be measured separately from data preservation.

For an explicitly source-backed view, add `--attachment-refs` with `--view
events`. It replaces standalone base64 image data URIs and Codex reasoning
`encrypted_content` strings with references only when the resulting serialized
page is smaller. Other strings and unknown encryption fields remain inline.
This is payload deferral, not self-contained lossless compression or image
understanding. No files are written and no URLs are fetched. The original source
must remain available; a source-backed reference is not an archival copy.

```bash
innen --format json read <session-id> --view events --compact --attachment-refs
innen --format json read <session-id> --offset <line-minus-one> \
  --attachment <pointer> --expect-sha256 <sha256>
```

The `innen.attachments.v1` envelope contains `page` and `attachments`. Each
reference has a zero-based page `record`, one-based source `line`, event-relative
JSON `pointer`, `kind`, UTF-8 `bytes`, and `sha256`. Decode the compact page first;
at each listed pointer, replace `{"attachment": index}` with the fetched string.
Only these listed locations are references; identical literal objects elsewhere
are untouched. Reuse the same source selection/root when retrieving. Attachment
lookup reads one event and returns the original string in `value`, including
any data URI prefix. Missing fields, wrong source rows or hash mismatches fail
explicitly. Appended history can remain compatible; edited/removed source data
can invalidate references. Actual fetch and multimodal access costs must be
counted separately from the smaller initial text response.

`--compact --deltas` optionally factors exact shared prefixes/suffixes of long
strings. This experimental `innen.edges.v1` envelope stores literal `fragments`
and a collision-free `join_key`. Replace each singleton object using that key
with the listed fragments concatenated in order, without a separator, then
decode the restored page's other encodings. The Rust compact decoder handles
this outer layer; attachment envelopes still require source-backed retrieval.
Fragments contain the exact changes, including negation and line endings.
References do not form chains. Candidates include adjacent distinct strings
in lexical order and exact repeated LF-line prefixes or whole lines inside
values; they are not inferred file versions or semantic relationships.
Encoded CLI packets include a `guide` explaining their format. The byte-size
comparison includes that guide and falls back to ordinary JSON if smaller.
Tokenizer savings and
model reading costs are separate measured properties. This flag does not change
the ordinary `--compact` default.

### Retrieval evidence and measurement scope

The public CLI contract preserves source line references and offers explicit
dialogue, context, and event views. Local development harnesses may compare
those views against immutable source captures, but their scripts, private
corpora, and tokenizer environments are outside this release. Any output-size
comparison must be read as a representation measurement; it does not establish
semantic completeness, comprehension quality, or billed-token savings.

Local development replays and representation checks are intentionally omitted
from the public release. They require excluded scripts, private captures, and
local tokenizer environments; they are not release evidence or billed-token
measurements.

### Local evaluation scope

Source-reader tests, benchmark reports, private transcript captures, and
tokenizer environments are maintained outside this public release. The public
contract is the CLI behavior and its source-provenance guarantees; local output
measurements are not claims about comprehension or billed-token savings.

### Everyday Agent Workflow

```bash
# 1. Pick up an unfinished task from prior agent session or inspect candidates
innen pickup
innen unfinished

# 2. Record or update a local progress checkpoint
innen checkpoint record --status active --objective "Refactor parser" --next-action "Fix tests"

# 3. Harvest incoming transcripts from agent chat sessions (dry-run check)
innen harvest --check

# 4. Ingest structured notes and append new knowledge to the journal
innen ingest

# 5. Retrieve relevant subgraph context (BM25 + 1~3 hop graph expansion)
innen query --q "clinical decision support" --format human

# 6. Inspect deterministic project rollup (Decisions, Tasks, Experiments)
innen project nhri-pcne

# 7. Full-text lexical search
innen search --keyword "medication error"

# 8. Verify repository health and reference integrity
innen doctor
innen lint

# 9. Rebuild local derived indexes from journal (~30ms)
innen index rebuild
```

---

## 7. Repository Layout

```text
crates/
  innen-core/     Core journal, Redb/Tantivy indexing, graph validation, query BFS, and tap engine
src/
  main.rs         Unified CLI driver
tests/
  cli_golden.rs   End-to-end golden parity test suite
```

---

## 8. License & Acknowledgments

- Grounded in the **LLM Wiki** philosophy pioneered by **Andrej Karpathy**.
- Licensed under the [MIT License](LICENSE).

## CI and Release Provenance

[The Rust CI workflow](.github/workflows/rust.yml) checks formatting and builds/tests
the workspace on GitHub-hosted Linux and macOS for pull requests and main pushes.
It uses the repository toolchain, read-only permissions, pinned actions, and
cancels superseded runs. Successful main or manual runs also build and smoke-test
optimized packages for macOS arm64 and Linux x86_64. Linux packages are built on
Ubuntu 24.04 and use the GNU runtime; they are not universal static Linux builds.

Releases include `SHA256SUMS` and per-target JSON build metadata binding each
archive to its source commit and GitHub Actions run. This metadata is not a
cryptographic attestation. Download the archive and `SHA256SUMS` from the same
release and verify the checksum before extracting.

The Homebrew workflow validates the release tag, archive, and published checksum
before updating the formula. It requires `HOMEBREW_TAP_TOKEN` for cross-repository
writes; if absent, it emits a notice and the verified formula must be updated
manually. This does not affect the release package checks.

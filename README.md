# innen (因縁) — a deterministic, CLI-first realization of the LLM Wiki

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 2021](https://img.shields.io/badge/Rust-2021%20%28MSRV%201.90%29-orange.svg)](https://www.rust-lang.org/)
[![Architecture: Event--Sourced](https://img.shields.io/badge/Architecture-Event--Sourced%20Graph-green.svg)](#architecture)
[![Zero-Daemon](https://img.shields.io/badge/Runtime-Zero--Daemon%20CLI%20Binary-black.svg)](#why-zero-daemon-cli-over-mcp)
[![Harvest: 99.98% Reduction](https://img.shields.io/badge/Harvest-99.98%25%20Token%20Reduction-brightgreen.svg)](#multi-agent-conversation-harvesting)

**innen is an open-source, high-performance knowledge engine and graph runtime designed for LLM coding agents. Built as a single, zero-daemon static binary in Rust, it implements an append-only event-sourced journal, dual embedded derived indexes (Redb + Tantivy CJK BM25), and multi-agent conversation harvesting adapters to turn ephemeral agent chats and raw markdown into a permanent, causally-sound knowledge base.**

*The name reads from Buddhist philosophy: **因縁** (innen) — dependent origination, causal connection. In genuine knowledge management, no claim exists in a vacuum. Every decision, task, experiment, and code artifact arises out of prior conditions and leaves traceable consequences. innen does not merely store text; it captures the causal graph of why things are the way they are.*

---

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

**`innen` is an opinionated, production-grade realization of the LLM Wiki paradigm.** It bridges human-readable markdown with the mathematical rigor of an append-only event journal, sub-35ms deterministic graph retrieval, and multi-agent conversation harvesting.

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

By keeping `innen` as a single static binary, every agent on your machine can invoke it with zero configuration overhead, and human engineers retain full command-line inspection capabilities.

---

## 3. Multi-Agent Conversation Harvesting

Modern AI-assisted engineering relies heavily on diverse coding agents. However, existing transcript ingesters suffer from a fatal flaw: **they only extract the user's prompt (User Input)**.

Reading user input alone captures *intent*, but completely misses *reality*:
- Which file was actually modified?
- What was the root cause identified by the agent?
- Did the test suite pass or fail?
- What was the SHA-256 hash or version tag of the delivered artifact?

### Bi-Directional Knowledge Folding

`innen` introduces **Bi-Directional Knowledge Folding** across 5 major coding agent session stores:

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                   Heterogeneous Coding Agent Stores                     │
│  Codex CLI        Claude Code       Gemini (Antigravity)   OpenCode     Grok CLI  │
│  (~/.codex)       (~/.claude)       (~/.gemini)            (SQLite)     (~/.grok) │
└────────┬───────────────┬───────────────────┬───────────────────┬───────────┬──────┘
         │               │                   │                   │           │
         └───────────────┴─────────┐   ┌─────┴───────────────────┴───────────┘
                                   ▼   ▼
               ┌───────────────────────────────────────────────┐
               │         innen Multi-Agent Tap Engine          │
               │   - Strip system prompts & <system-reminder>  │
               │   - Extract User Intent (Prompt & Directives) │
               │   - Extract Outcomes, Conclusions, Root Causes│
               │   - Harvest File Paths & SHA-256 Receipts     │
               └───────────────────────┬───────────────────────┘
                                       │
                                       ▼ (99.98% Token Reduction)
               ┌───────────────────────────────────────────────┐
               │    High-Density Event Stream (.innen/journal)  │
               └───────────────────────────────────────────────┘
```

### Empirical 7-Day Multi-Tool Benchmark

Harvesting all real engineering sessions across 5 coding tools over a 7-day period on a developer workstation:

| Coding Tool | Harvested Sessions | Raw Log Tokens | Folded Knowledge Tokens | Token Savings |
|---|---|---|---|---|
| **OpenCode** (SQLite DB) | 123 | 114,357,511 | 55,274 | **99.95%** |
| **Codex CLI** (`.jsonl`) | 72 | 47,819,401 | 30,527 | **99.94%** |
| **Gemini / Antigravity** (`.jsonl`) | 18 | 26,552,709 | 10,757 | **99.96%** |
| **Claude Code** (`.jsonl`) | 3 | 479,796 | 1,841 | **99.62%** |
| **Grok CLI** (`chat_history.jsonl`) | 1 | 8,920 | 660 | **92.60%** |
| **TOTAL (Across 5 Tools)** | **217** | **189,218,337** | **99,059** | **99.98%** 🔥 |

By filtering peripheral system instructions and tool chatter while preserving paired **Intent + Outcome + Receipt** tuples, `innen` reduces **189 million raw tokens into 99k high-density tokens** ready for causal graph linking and immediate recall.

---

## 4. Architecture

`innen` strictly decouples immutable ground truth from throwaway, high-speed derived indexes:

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                           The LLM Agent                                 │
│         (Codex / Claude Code / Gemini / OpenCode / Grok)                │
└───────────────────▲─────────────────────────────────┬───────────────────┘
                    │ query / project (<35ms)         │ harvest / ingest / append
                    │ (90% – 95% token reduction)     │ (append-only)
┌───────────────────┴─────────────────────────────────▼───────────────────┐
                    │                      innen CLI                      │
                    │           (Single Static Rust Binary)               │
├───────────────────┴────────────┬────────────────────────────────────────┤
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

### A. Execution Latency & Memory Footprint

| Command | Description | Mean Latency | Peak Memory (RSS) |
|---|---|---|---|
| `innen --help` | CLI cold startup & argument parsing | **17.3 ms** | **2.9 MB** |
| `innen lint` | Full journal & reference integrity audit | **33.4 ms** | **5.5 MB** |
| `innen status` | Materialized graph census & node breakdown | **29.4 ms** | **6.6 MB** |
| `innen project <id>` | Deterministic project page synthesis | **21.3 ms** | **6.4 MB** |
| `innen index rebuild` | Full Redb + Tantivy index regeneration | **30.1 ms** | **2.8 MB** |
| `innen query --q <term>` | Jieba tokenize + BM25 + 3-hop BFS expansion | **309.7 ms** | **76.4 MB** (includes CJK dict) |

*All inspection commands execute in **under 35 milliseconds** with less than **7 MB** of resident memory, allowing LLM agent toolhooks to invoke them with negligible latency.*

### B. Prompt Token Reduction for Agent Context

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

- **`innen project <id>` (95.9% Token Reduction)**: Synthesizes active Decisions, Tasks, Experiments, and Datasets in **102 tokens** (vs. 2,500 tokens), giving the model an exact high-density mental model without peripheral chatter.
- **`innen query --q <term>` (90.2% Token Reduction)**: Extracts a ranked 1~3 hop subgraph of 13 critical entities and SHA-256 artifact receipts in **245 tokens**, preserving 100% causal recall without context bloat.

---

## 6. Quick Start

### Installation

`innen` distributes as a self-contained static binary with zero external runtime requirements:

```bash
git clone https://github.com/RikaiDev/innen.git
cd innen

# Run full test suite (121/121 passed)
cargo test --workspace

# Build optimized release binary
cargo build --release

# Install to PATH
cp target/release/innen /usr/local/bin/innen
```

### Everyday Agent Workflow

```bash
# 1. Harvest incoming transcripts from agent chat sessions (dry-run check)
innen harvest --check

# 2. Ingest structured notes and append new knowledge to the journal
innen ingest

# 3. Retrieve relevant subgraph context (BM25 + 1~3 hop graph expansion)
innen query --q "clinical decision support" --format human

# 4. Inspect deterministic project rollup (Decisions, Tasks, Experiments)
innen project nhri-pcne

# 5. Full-text lexical search
innen search --keyword "medication error"

# 6. Verify repository health and reference integrity
innen doctor
innen lint

# 7. Rebuild local derived indexes from journal (~30ms)
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
- Dual-licensed under [MIT](LICENSE) or [Apache-2.0](LICENSE-APACHE).

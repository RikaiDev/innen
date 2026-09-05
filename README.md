# innen (因縁) — a deterministic realization of the LLM Wiki

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust: 2021](https://img.shields.io/badge/Rust-2021%20%28MSRV%201.90%29-orange.svg)](https://www.rust-lang.org/)
[![Architecture: Event--Sourced](https://img.shields.io/badge/Architecture-Event--Sourced%20Graph-green.svg)](#architecture)
[![Zero-Daemon](https://img.shields.io/badge/Runtime-Zero--Daemon%20Static%20Binary-black.svg)](#quick-start)

**innen is an open-source, high-performance knowledge engine and graph runtime designed for LLM agents. It implements an event-sourced journal, dual embedded derived indexes (Redb + Tantivy CJK BM25), and hybrid subgraph retrieval to turn raw markdown files into a living, causally-sound, persistent knowledge base.**

*The name reads from Buddhist philosophy: **因縁** (innen) — dependent origination, causal connection. In genuine knowledge management, no claim exists in a vacuum. Every decision, task, experiment, and document arises out of prior conditions and leaves traceable consequences. innen does not merely store text; it captures the causal graph of why things are the way they are.*

---

## 1. Context & Motivation

In April 2026, **Andrej Karpathy** published his formal [LLM Wiki Guide](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f), outlining a radical shift away from opaque corporate AI memories toward an open, explicit, compounding knowledge base:

> *"In the age of LLM agents, sharing specific code or apps is no longer necessary. You only need to share the core idea (an idea file), and everyone's personal agent will tailor-build the realization to their exact needs."*

Karpathy articulated four foundational pillars of the **LLM Wiki**:
- **Explicit**: The knowledge is laid out in plain sight; you can inspect exactly what the AI knows and what it does not.
- **Yours**: Held on your own machine under your direct ownership, free from vendor lock-in.
- **File over app**: Stored in durable formats (Markdown, JSON, images) readable by standard Unix tools.
- **BYOAI (Bring Your Own AI)**: Model-agnostic; agents from Claude, Gemini, Codex, to local weights can all read and write to it.

### The Limits of Naive Markdown at Scale

Karpathy's initial observation was elegant: *"At small scale (~100 articles, ~400k words), no RAG is needed; the LLM simply navigates through index files."*

However, once an agentic knowledge base scales to hundreds of daily transcripts, multi-project experiment matrixes, compliance receipts, and evolving research state, naive flat markdown approaches hit a structural ceiling:

1. **Context Window & Cost Explosion**: An agent needing context on a project must dump raw transcripts and verbose ledgers into its prompt (2,500 – 5,000+ tokens per call). This rapidly inflates API costs, degrades inference speed, and causes context dilution / needle-in-a-haystack forgetting.
2. **Loss of Temporal Provenance**: Flat file edits overwrite past state. When was a decision made? Which experiment invalidated it? What was the historical baseline? Overwrites destroy causal continuity.
3. **Dangling References & Silent Drift**: Without schema validation and reference integrity, broken links, orphan notes, and phantom projects proliferate undetected.
4. **Vector Database Pitfalls**: Offloading knowledge to opaque vector databases introduces non-deterministic recall, loses exact identifier matching (e.g. SHA-256 hashes or issue codes), and requires heavy, battery-draining background daemon processes.

**`innen` is an opinionated, production-grade realization of the LLM Wiki paradigm.** It bridges human-readable markdown with the mathematical rigor of an append-only event journal and sub-35ms deterministic graph retrieval.

---

## 2. Architecture

`innen` decouples immutable ground truth from throwaway, high-speed derived indexes:

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                           The LLM Agent                                 │
│        (Claude Code / Antigravity / Codex / Local Open Weights)         │
└───────────────────▲─────────────────────────────────┬───────────────────┘
                    │ query / project (<35ms)         │ ingest / append
                    │ (90% – 95% token reduction)     │ (append-only)
┌───────────────────┴─────────────────────────────────▼───────────────────┐
│                              innen CLI                                  │
│                 (Single 12MB Rust Static Binary)                        │
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
  Every entity (`node.upsert`), relation (`edge.assert`), and cancellation (`edge.retract`) is recorded as an immutable JSON line in `.innen/journal.jsonl`. Edits never erase history; retraction preserves the historical record while updating the materialized present.
- **Bi-Temporal Modeling**:
  Events record both `observed_utc` (when the system recorded the event) and `valid_from` / `valid_to` (when the fact was true in reality). Queries can traverse historical snapshots using `--as-of <TIMESTAMP>`.
- **Zero-Daemon Dual Indexes**:
  - **Redb**: Embedded, pure-Rust transactional key-value store maintaining materialized node tables and adjacency graphs.
  - **Tantivy CJK FTS**: Embedded full-text search engine with bundled Jieba CJK segmentation. No background services, no open ports, zero cloud dependencies.
- **Instant Index Rebuilds**:
  Indexes are strictly derived state. Corrupted, stale, or deleted indexes can be completely re-materialized from the journal via `innen rebuild` in **under 35 milliseconds**.
- **Deterministic Integrity (`doctor` / `lint`)**:
  Zero-mutation health checks ensure all edge endpoints resolve, quarantine directories remain empty, and journals parse flawlessly, returning strict exit codes (`0` for clean, `1` for quarantined, `2` for corruption/dangling edges).

---

## 3. Empirical Benchmarks

Tested on a production research knowledge base comprising **635 events, 395 relational edges, 85 structured wiki pages, and 11 active project ledgers** on Apple Silicon:

### A. Execution Latency & Memory Footprint

| Command | Description | Mean Latency | Peak Memory (RSS) |
|---|---|---|---|
| `innen --help` | CLI cold startup & argument parsing | **17.3 ms** | **2.9 MB** |
| `innen lint` | Full journal & reference integrity audit | **33.4 ms** | **5.5 MB** |
| `innen status` | Materialized graph census & node breakdown | **29.4 ms** | **6.6 MB** |
| `innen project <id>` | Deterministic project page synthesis | **21.3 ms** | **6.4 MB** |
| `innen query --q <term>` | Jieba tokenize + BM25 + 3-hop BFS expansion | **309.7 ms** | **76.4 MB** (includes CJK dict) |
| `innen rebuild` | Full Redb + Tantivy index regeneration | **30.1 ms** | **2.8 MB** |

*All core inspection commands execute in **under 35 milliseconds** with less than **7 MB** of resident memory, allowing LLM agent toolhooks to invoke them with negligible overhead.*

### B. Prompt Token Reduction for Agent Context

When an LLM agent investigates a specific project (e.g. healthcare research state), different retrieval strategies yield dramatically different prompt token loads:

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

## 4. Quick Start

### Installation

`innen` distributes as a self-contained 12MB static binary with zero external runtime requirements:

```bash
git clone https://github.com/RikaiDev/innen.git
cd innen

# Run full test suite (121/121 passed)
cargo test --workspace

# Build optimized release binary
cargo build --release

# Install locally
cp target/release/innen /usr/local/bin/innen
```

### Everyday Agent Workflow

```bash
# 1. Harvest incoming transcripts from agent chat sessions
innen harvest --check

# 2. Ingest structured notes and append new knowledge to the journal
innen ingest

# 3. Retrieve relevant subgraph context (BM25 + 1~3 hop graph expansion)
innen query --q "clinical decision support" --format human

# 4. Inspect deterministic project rollup (Decisions, Tasks, Experiments)
innen project nhri-pcne

# 5. Full-text lexical search
innen search "medication error"

# 6. Verify repository health and reference integrity
innen doctor
innen lint

# 7. Rebuild local derived indexes from journal (takes ~30ms)
innen rebuild
```

---

## 5. Repository Layout

```text
crates/
  innen-core/     Core journal, Redb/Tantivy indexing, graph validation, and query BFS
  innen-mcp/      Model Context Protocol (MCP) stdio server interface
src/
  main.rs         Unified CLI driver
tests/
  cli_golden.rs   End-to-end golden parity test suite
```

---

## 6. License & Acknowledgments

- Grounded in the **LLM Wiki** philosophy pioneered by **Andrej Karpathy**.
- Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).\n
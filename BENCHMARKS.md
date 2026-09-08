# Benchmark and token-usage claims

innen exists to reduce the context and tokens an agent must reload for a task
without silently dropping status, evidence boundaries, corrections, or unknowns.
This document separates what has been measured from what still needs evidence.

## Historical paired output measurement

A fixed snapshot measured the default project brief against two evidence views
using `o200k_base` reference-token counts:

| View | Tokens | Brief reduction |
|---|---:|---:|
| Complete evidence view | 21,758 | — |
| Same-task-set evidence view | 16,564 | — |
| Default project brief | 2,046 | 90.60% vs complete; 87.65% vs same-task-set |

The brief and evidence views contained the same 28 non-terminal task rows, and
the brief's displayed fields matched those rows. This is a historical
output-length result, not a v0.3.0 performance guarantee.

## What the percentage does and does not mean

It measures serialized output tokens for one fixed workload. It does not by
itself measure:

- provider-billed input, cached input, output, or reasoning tokens;
- total session usage including instructions, tool schemas, commands, retries,
  and follow-up evidence expansion;
- semantic equivalence, recall, answer correctness, or model comprehension;
- another corpus, tokenizer, project shape, or innen release.

## Required current-version protocol

A publishable release benchmark must record:

1. innen version and binary SHA-256;
2. a public or independently reproducible fixed fixture and its SHA-256;
3. exact commands, views, pagination, and tokenizer/version;
4. paired task/field coverage before comparing output length;
5. brief and expanded-output token counts;
6. any evidence expansions and total agent/provider usage as separate metrics;
7. negative cases for missing, conflicting, and authorization-sensitive facts.

Until that protocol is rerun for v0.3.0, use the historical percentages only
with the qualifier above. Do not translate them into cost savings.

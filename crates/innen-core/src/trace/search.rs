use super::cache::{graph, kind};
use super::graph::{candidates, Candidate, Hop};
use super::lexical::{bm25, tokens};
use super::source::{excerpt, resolve_locator, source, SourceCache, SourceFailure};
use super::types::{Error, Locator, Options};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

struct RankedHit {
    score: f64,
    node: String,
    line: usize,
    value: Value,
}
#[derive(Default)]
struct Results {
    hits: Vec<RankedHit>,
    reports: Vec<Value>,
    matched: usize,
    attempted: usize,
    measured_bytes: u64,
    charged_bytes: u64,
    unknown_reads: usize,
    scanned: usize,
    cache_hits: usize,
    remote: usize,
    projection_incomplete: bool,
    metadata_limited: bool,
    metadata_followed: usize,
}

fn compare(a: &RankedHit, b: &RankedHit) -> std::cmp::Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| a.node.cmp(&b.node))
        .then_with(|| a.line.cmp(&b.line))
}
fn validate(opts: &Options) -> Result<Vec<String>, Error> {
    if opts.limit == 0
        || opts.limit > 100
        || opts.max_sources == 0
        || opts.max_sources > 64
        || opts.max_nodes == 0
        || opts.max_nodes > 4096
        || opts.max_depth > 8
        || opts.max_bytes == 0
        || opts.max_bytes > 512 * 1024 * 1024
        || opts.max_records == 0
        || opts.max_records > 200_000
    {
        return Err(Error("invalid retrieval budgets".into()));
    }
    if opts
        .role
        .as_deref()
        .is_some_and(|r| !matches!(r, "user" | "assistant" | "source"))
    {
        return Err(Error("role must be user, assistant, or source".into()));
    }
    let query = tokens(&opts.q);
    if query.is_empty() {
        return Err(Error("query has no searchable terms".into()));
    }
    Ok(query)
}

struct Evidence<'a> {
    candidate: &'a Candidate,
    data: &'a SourceCache,
    loc: &'a Locator,
    key: &'a str,
    rank: usize,
    cached: bool,
}

impl Results {
    fn admits(&self, score: f64, node: &str, line: usize, limit: usize) -> bool {
        let Some(worst) = self.hits.last() else {
            return true;
        };
        self.hits.len() < limit
            || score > worst.score
            || (score == worst.score && (node, line) < (worst.node.as_str(), worst.line))
    }

    fn rank(&mut self, root: &Path, opts: &Options, query: &[String], e: &Evidence<'_>) -> usize {
        let mut withheld = 0;
        for (rank, (i, bm25_score)) in bm25(&e.data.docs, query).into_iter().enumerate() {
            let passage = &e.data.passages[i];
            if opts.role.as_ref().is_some_and(|r| r != &passage.role) {
                continue;
            }
            self.matched += 1;
            if !crate::tap::scan_credentials(&passage.text).is_empty() {
                withheld += 1;
                continue;
            }
            let score = 1.0 / (60.0 + e.rank as f64) + 1.0 / (61.0 + rank as f64);
            if !self.admits(score, &e.candidate.node, passage.line, opts.limit) {
                continue;
            }
            let value = json!({
                "source_node":e.candidate.node,"seed_node":e.candidate.seed,"path":e.candidate.path,
                "source":e.loc,"source_path":e.data.source_path,"line":passage.line,"end_line":passage.end_line,
                "role":passage.role,"role_basis":if matches!(e.loc,Locator::Conversation{..}){"native_metadata"}else{"source_format_heading_or_plain_text"},
                "timestamp":passage.timestamp,"quote":excerpt(&passage.text,query),
                "record_sha256":passage.hash,"source_sha256":e.data.source_sha256,"source_complete":e.data.complete,
                "source_freshness":if e.cached{"metadata_checked_cached_bytes"}else{"read_this_query"},
                "source_version":e.data.version,"bm25":bm25_score,"rrf":score,"graph_score":e.candidate.score,
                "interpretation":"source text; role is not proof of a decision",
                "expand_argv":["innen","--root",root.display().to_string(),"trace","--expand",e.key,
                    "--record",i.to_string(),"--expect-sha256",passage.hash]
            });
            self.hits.push(RankedHit {
                score,
                node: e.candidate.node.clone(),
                line: passage.line,
                value,
            });
            self.hits.sort_by(compare);
            self.hits.truncate(opts.limit);
        }
        withheld
    }

    fn failure(
        &mut self,
        candidate: &Candidate,
        loc: &Locator,
        error: SourceFailure,
        remaining: u64,
    ) {
        let charged = error.consumed.unwrap_or(remaining).min(remaining);
        self.charged_bytes += charged;
        if let Some(measured) = error.consumed {
            self.measured_bytes += measured;
        } else {
            self.unknown_reads += 1;
        }
        self.reports.push(json!({"node_id":candidate.node,"status":"unavailable","reason":error.to_string(),
            "locator":loc,"path":candidate.path,"source_bytes_charged":charged,"source_bytes_measured":error.consumed}));
    }

    fn success(&mut self, e: &Evidence<'_>, read: u64, withheld: usize) {
        self.measured_bytes += read;
        self.charged_bytes += read;
        self.projection_incomplete |= e.data.projection_incomplete;
        if e.cached {
            self.cache_hits += 1;
        } else {
            self.scanned += e.data.records_scanned;
        }
        self.reports.push(json!({"node_id":e.candidate.node,"status":if e.data.complete{"searched"}else{"partial"},
            "path":e.candidate.path,"cache_hit":e.cached,"source_bytes_read":read,
            "records_scanned":if e.cached{0}else{e.data.records_scanned},"warnings":e.data.warnings,
            "projection_incomplete":e.data.projection_incomplete,"credential_records_withheld":withheld}));
    }
}

fn enqueue_metadata(
    candidate: &Candidate,
    data: &SourceCache,
    opts: &Options,
    pending: &mut Vec<Candidate>,
    results: &mut Results,
) {
    if data.references.is_empty() {
        return;
    }
    if candidate.path.len() >= opts.max_depth {
        results.metadata_limited = true;
        return;
    }
    for reference in &data.references {
        if pending.len() >= 4096 {
            results.metadata_limited = true;
            break;
        }
        let id = format!(
            "source-ref:{}",
            crate::ids::sha256_hex(
                &serde_json::to_vec(&reference.locator).expect("locator serializes")
            )
        );
        let mut path = candidate.path.clone();
        path.push(Hop {
            from: candidate.node.clone(),
            edge: "METADATA_SESSION_REF".into(),
            to: id.clone(),
            direction: "forward".into(),
            provenance: Some(format!(
                "sha256:{}#{}",
                data.source_sha256, reference.pointer
            )),
        });
        pending.push(Candidate {
            node: id,
            seed: candidate.seed.clone(),
            score: candidate.score * 0.9,
            path,
            locator: reference.locator.clone(),
        });
        results.metadata_followed += 1;
    }
}

fn retrieve(
    root: &Path,
    opts: &Options,
    query: &[String],
    mut pending: Vec<Candidate>,
) -> Result<(Results, usize, bool), Error> {
    let mut out = Results::default();
    let mut catalog = BTreeMap::new();
    let mut catalog_count = 0;
    let mut seen = BTreeSet::new();
    let mut limited = false;
    while !pending.is_empty() {
        pending.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.node.cmp(&b.node))
        });
        let candidate = pending.remove(0);
        if matches!(candidate.locator, Locator::Url { .. }) {
            out.remote += 1;
            if out.remote <= 3 {
                out.reports.push(
                    json!({"node_id":candidate.node,"status":"remote_reference_not_fetched",
                "locator":candidate.locator,"path":candidate.path}),
                );
            }
            continue;
        }
        if out.attempted >= opts.max_sources {
            limited = true;
            break;
        }
        let loc = match resolve_locator(&candidate.locator, opts, &mut catalog, &mut catalog_count)
        {
            Ok(loc) => loc,
            Err(error) => {
                out.attempted += 1;
                out.reports
                    .push(json!({"node_id":candidate.node,"status":"unresolved",
                "reason":error.to_string(),"path":candidate.path}));
                continue;
            }
        };
        if !seen.insert(serde_json::to_string(&loc)?) {
            continue;
        }
        out.attempted += 1;
        let remaining = opts.max_bytes.saturating_sub(out.charged_bytes);
        match source(root, &loc, opts, remaining) {
            Err(error) => out.failure(&candidate, &loc, error, remaining),
            Ok((data, key, cached, read)) => {
                let e = Evidence {
                    candidate: &candidate,
                    data: &data,
                    loc: &loc,
                    key: &key,
                    rank: out.attempted,
                    cached,
                };
                let withheld = out.rank(root, opts, query, &e);
                out.success(&e, read, withheld);
                enqueue_metadata(&candidate, &data, opts, &mut pending, &mut out);
            }
        }
    }
    Ok((out, catalog_count, limited))
}

fn coverage(
    results: &Results,
    graph_limited: bool,
    source_limited: bool,
    wiki_missing: bool,
) -> (bool, Vec<String>) {
    let mut warnings = Vec::new();
    if wiki_missing {
        warnings.push("wiki_projection_missing: run innen wiki sync".to_string());
    }
    if graph_limited {
        warnings.push("graph_node_depth_or_frontier_budget_limited".into());
    }
    if source_limited {
        warnings.push("source_candidate_budget_limited".into());
    }
    if results.remote > 0 {
        warnings.push(format!(
            "{} remote references not fetched; at most three examples shown",
            results.remote
        ));
    }
    if results.projection_incomplete {
        warnings.push("source_projection_incomplete".into());
    }
    if results.metadata_limited {
        warnings.push("metadata_source_path_budget_limited".into());
    }
    let unavailable = results.reports.iter().any(|r| {
        matches!(
            r["status"].as_str(),
            Some("unavailable" | "unresolved" | "partial")
        )
    });
    (
        wiki_missing
            || graph_limited
            || source_limited
            || unavailable
            || results.remote > 0
            || results.projection_incomplete
            || results.metadata_limited,
        warnings,
    )
}

/// Coordinate independently bounded graph retrieval, source loading, and ranking.
pub fn search(root: &Path, opts: &Options) -> Result<Value, Error> {
    let query = validate(opts)?;
    let (graph, graph_cache_hit, graph_bytes) = graph(root, opts.refresh)?;
    let (sources, visited, graph_limited) = candidates(&graph, root, opts, &query)?;
    let (results, catalog_count, source_limited) = retrieve(root, opts, &query, sources)?;
    let wiki_missing = !graph.nodes.values().any(|n| kind(n) == "wiki");
    let (incomplete, mut warnings) =
        coverage(&results, graph_limited, source_limited, wiki_missing);
    if catalog_count > 0 {
        warnings.push("native_identity_catalog_metadata_IO_not_in_source_byte_counter".into());
    }
    let found = !results.hits.is_empty();
    let measured = if results.unknown_reads == 0 {
        Some(results.measured_bytes)
    } else {
        None
    };
    Ok(json!({
        "resolution":if found{"evidence_candidates"}else{"no_match_in_covered_sources"},"query":opts.q,
        "algorithm":"BM25 + bounded typed provenance best-first traversal + local BM25/RRF(k=60)",
        "journal_sha256":graph.journal_sha256,"hits":results.hits.into_iter().map(|h|h.value).collect::<Vec<_>>(),
        "total_matched_passages":results.matched,"output_truncated":results.matched>opts.limit,
        "coverage_incomplete":incomplete,"warnings":warnings,"sources":results.reports,
        "work":{"graph_cache_hit":graph_cache_hit,"graph_or_cache_bytes":graph_bytes,"nodes_visited":visited,
            "sources_attempted":results.attempted,"source_cache_hits":results.cache_hits,
            "source_bytes_read":measured,"source_bytes_charged":results.charged_bytes,
            "source_records_scanned":results.scanned,"failed_reads_with_unknown_consumption":results.unknown_reads,
            "catalog_candidates_observed":catalog_count,"metadata_references_followed":results.metadata_followed},
        "metrics":{"source_bytes_read":"logical scanner-consumed bytes; null when any failed scan has unknown consumption; excludes locate/stat/read-ahead/cache I/O"},
        "limits":{"max_sources":opts.max_sources,"max_bytes":opts.max_bytes,"max_records_per_source":opts.max_records,
            "max_nodes":opts.max_nodes,"max_depth":opts.max_depth},
        "boundary":"Graph routes are retrieval evidence, not inferred ownership or semantic authority; no match is not proof of absence."
    }))
}

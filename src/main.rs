//! P1+P2 CLI dispatch (Tasks 8d, 12).
//!
//! Thin dispatch: parse → core calls → print. No business logic here.
//!
//! KB root resolution (pinned): `--root <dir>` flag wins, else `INNEN_ROOT`
//! env (non-empty), else current working directory.
//!
//! Output format: global `--format human|json` (default `json`). Explicit
//! only — no TTY sniffing, no env fallback for rendering. Deterministic:
//! `--format json` is compact `serde_json::to_string` (single line +
//! trailing newline); struct field order is the wire order.
//!
//! P2 commands (Task 12): `guide`, `search --keyword --limit`, `status`,
//! `timeline [filter]`, `project <id>`, `profile`, `artifact add --file`,
//! `cloud status|doctor`. P2 JSON shapes (field order is wire order):
//! guide `{"text"}`, search `{"hits":[{node_id,excerpt}]}`,
//! status `{"nodes","edges","events","artifacts"}`,
//! timeline `{"entries":[{observed_utc,op,summary}]}`,
//! project `{"id","render"}`, profile `{"render"}`,
//! artifact `{"sha256","path","bytes"}`,
//! cloud status `{"remote","output"}`, cloud doctor `{"output"}`.
//! P2 human renders raw markdown for project/profile (byte-exact via
//! `print!`, no extra newline) and TSV tables elsewhere; P2 errors print
//! the core message raw to stderr (no `error:` prefix) so
//! `unknown project: <id>` matches byte-for-byte.

use std::path::PathBuf;

use clap::{CommandFactory as _, Parser, Subcommand};

/// KB root resolution: `--root` flag > `INNEN_ROOT` env (non-empty) > cwd.
fn resolve_root(cli_root: Option<PathBuf>) -> PathBuf {
    if let Some(r) = cli_root {
        return r;
    }
    if let Ok(s) = std::env::var("INNEN_ROOT") {
        if !s.trim().is_empty() {
            return PathBuf::from(s);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[derive(Parser)]
#[command(
    name = "innen",
    version,
    about = "innen P1+P2 knowledge CLI",
    long_about = "innen P1+P2 knowledge CLI.\n\nKB root resolution: --root <dir> > INNEN_ROOT env (non-empty) > cwd."
)]
struct Cli {
    /// Output format (explicit only; no TTY sniffing). Applies to all commands.
    #[arg(long, global = true, default_value = "json", value_parser = ["json", "human"])]
    format: String,
    /// KB root dir. Precedence: --root > INNEN_ROOT env > cwd.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Ranked query over the journal (replay + FTS + graph BFS).
    Query(QueryArgs),
    /// Typed graph writes (journal appends).
    Graph {
        #[command(subcommand)]
        op: GraphOp,
    },
    /// Derived index maintenance.
    Index {
        #[command(subcommand)]
        op: IndexOp,
    },
    /// Report-only health checks (exit 0/1/2).
    Doctor,
    /// Report-only lint subset (exit 0/1/2).
    Lint,
    /// Persisted config get/set (machine.json layer).
    Config {
        #[command(subcommand)]
        op: ConfigOp,
    },
    /// Print shell completions to stdout.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_parser = ["bash", "zsh", "fish", "powershell"])]
        shell: String,
    },
    /// Short navigation text (Task 9 core `guide_text`).
    Guide,
    /// Lexical-only search over label/body (Task 9 core `search`).
    Search(SearchArgs),
    /// Journal counts (Task 9 core `status`).
    Status,
    /// Journal history in order (Task 9 core `timeline`).
    Timeline(TimelineArgs),
    /// Project page render grouped by member kind (Task 10).
    Project(ProjectArgs),
    /// Profile page render from profile/profile.toml (Task 10).
    Profile,
    /// Content-addressed artifact writes (Task 11a).
    Artifact {
        #[command(subcommand)]
        op: ArtifactOp,
    },
    /// Stubbed rclone cloud harness (Task 11b; `rclone` via PATH lookup).
    Cloud {
        #[command(subcommand)]
        op: CloudOp,
    },
}

#[derive(clap::Args)]
struct QueryArgs {
    /// Query string (FTS over label/body UNION exact node-id substring).
    #[arg(long)]
    q: String,
    /// As-of cutoff `YYYY-MM-DDTHH:MM:SSZ` (defaults to now).
    #[arg(long = "as-of")]
    as_of: Option<String>,
    /// Max hits (default 20, max 100; >100 clamps with `truncated`).
    #[arg(long, default_value_t = 20)]
    limit: u16,
    /// Bypass validity filtering (keep expired edges).
    #[arg(long = "include-expired")]
    include_expired: bool,
}

#[derive(Subcommand)]
enum GraphOp {
    /// Append a node.upsert event.
    Node(GraphNodeArgs),
    /// Append an edge.assert event (adjacency-validated).
    Relate(GraphRelateArgs),
    /// Append an edge.retract event.
    Retract(GraphRetractArgs),
}

#[derive(clap::Args)]
struct GraphNodeArgs {
    /// Node id.
    #[arg(long)]
    id: String,
    /// Node kind (NodeType display name; unknown stays Custom).
    #[arg(long, visible_alias = "type")]
    kind: String,
    /// Human label.
    #[arg(long)]
    label: String,
    /// Optional body text.
    #[arg(long)]
    body: Option<String>,
    /// Provenance (required when kind is custom).
    #[arg(long)]
    provenance: Option<String>,
}

#[derive(clap::Args)]
struct GraphRelateArgs {
    /// From node id.
    #[arg(long)]
    from: String,
    /// Edge type (SCREAMING_SNAKE_CASE; unknown stays Custom).
    #[arg(long, visible_alias = "type")]
    edge: String,
    /// To node id or URI (`/abs` or `scheme://...` only under LOCATED_AT/ORIGINATED_AT).
    #[arg(long)]
    to: String,
    /// Optional weight.
    #[arg(long)]
    weight: Option<f32>,
    /// Optional valid-from `YYYY-MM-DDTHH:MM:SSZ`.
    #[arg(long = "valid-from")]
    valid_from: Option<String>,
    /// Optional valid-until `YYYY-MM-DDTHH:MM:SSZ`.
    #[arg(long = "valid-until")]
    valid_until: Option<String>,
    /// Provenance (required for custom parties).
    #[arg(long)]
    provenance: Option<String>,
}

#[derive(clap::Args)]
struct GraphRetractArgs {
    /// From node id.
    #[arg(long)]
    from: String,
    /// Edge type.
    #[arg(long, visible_alias = "type")]
    edge: String,
    /// To node id.
    #[arg(long)]
    to: String,
}

#[derive(Subcommand)]
enum IndexOp {
    /// Drop derived files and rebuild from the journal.
    Rebuild,
}

#[derive(Subcommand)]
enum ConfigOp {
    /// Print one resolved config value as JSON.
    Get {
        /// Key: root | format | rebuild_on_open.
        key: String,
    },
    /// Persist one key into .innen/machine.json.
    Set {
        /// Key: root | format | rebuild_on_open.
        key: String,
        /// Value to store.
        value: String,
    },
}

/// P2 `search --keyword <kw> [--limit <n>]`: lexical-only, no graph walk.
#[derive(clap::Args)]
struct SearchArgs {
    /// Keyword (tantivy query over label/body; empty matches nothing).
    #[arg(long)]
    keyword: String,
    /// Max hits (default 20).
    #[arg(long, default_value_t = 20)]
    limit: u16,
}

/// P2 `timeline [filter]`: journal order; filter is a `YYYY[-MM]` prefix.
#[derive(clap::Args)]
struct TimelineArgs {
    /// Optional prefix filter, e.g. `2026-09`.
    filter: Option<String>,
}

/// P2 `project <id>`: project page render.
#[derive(clap::Args)]
struct ProjectArgs {
    /// Project node id.
    id: String,
}

#[derive(Subcommand)]
enum ArtifactOp {
    /// Store a file content-addressed + record artifact node/edge.
    Add(ArtifactAddArgs),
}

/// P2 `artifact add --file <path> [--project <id>]`.
#[derive(clap::Args)]
struct ArtifactAddArgs {
    /// Source file to store.
    #[arg(long)]
    file: PathBuf,
    /// Optional project id for a BELONGS_TO edge.
    #[arg(long)]
    project: Option<String>,
}

#[derive(Subcommand)]
enum CloudOp {
    /// `rclone lsd <remote>:` (canned `canned-dir` under the test stub).
    Status(CloudStatusArgs),
    /// `rclone version` (stub reports canned version).
    Doctor,
}

/// P2 `cloud status [--remote <name>]` (default `myremote` so bare
/// `cloud status` works against the stub).
#[derive(clap::Args)]
struct CloudStatusArgs {
    /// Remote name (stub ignores the value).
    #[arg(long, default_value = "myremote")]
    remote: String,
}

// --- P2 JSON wire structs (field order is the wire order) ---

#[derive(serde::Serialize)]
struct GuideJson {
    text: String,
}

#[derive(serde::Serialize)]
struct SearchHitJson {
    node_id: String,
    excerpt: String,
}

#[derive(serde::Serialize)]
struct SearchJson {
    hits: Vec<SearchHitJson>,
}

#[derive(serde::Serialize)]
struct StatusJson {
    nodes: u64,
    edges: u64,
    events: u64,
    artifacts: u64,
}

#[derive(serde::Serialize)]
struct TimelineEntryJson {
    observed_utc: String,
    op: String,
    summary: String,
}

#[derive(serde::Serialize)]
struct TimelineJson {
    entries: Vec<TimelineEntryJson>,
}

#[derive(serde::Serialize)]
struct ProjectJson {
    id: String,
    render: String,
}

#[derive(serde::Serialize)]
struct ProfileJson {
    render: String,
}

#[derive(serde::Serialize)]
struct ArtifactJson {
    sha256: String,
    path: String,
    bytes: u64,
}

#[derive(serde::Serialize)]
struct CloudStatusJson {
    remote: String,
    output: String,
}

#[derive(serde::Serialize)]
struct CloudDoctorJson {
    output: String,
}

fn is_human(format: &str) -> bool {
    format == "human"
}

/// TSV field budget (chars, not bytes — CJK safe) for human tables.
const TSV_FIELD_LIMIT: usize = 200;

/// Escape one human-table TSV field: truncate to [`TSV_FIELD_LIMIT`] chars
/// (CJK-safe, chars not bytes) with a `…` marker, then encode `\t`→`\\t`,
/// `\n`→`\\n`, `\r`→`\\r` so embedded tabs/newlines cannot break rows.
/// Truncation runs first so escape sequences stay intact.
fn escape_tsv_field(s: &str) -> String {
    let truncated: String = if s.chars().count() > TSV_FIELD_LIMIT {
        let mut out: String = s.chars().take(TSV_FIELD_LIMIT).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    };
    truncated
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn print_query_json(out: &innen_core::query::QueryOutput) {
    println!(
        "{}",
        serde_json::to_string(out).expect("query output serializes")
    );
}

fn print_query_human(out: &innen_core::query::QueryOutput) {
    // Bounded P1 human table: id/kind/score/why (+ label for readability).
    // All string fields are TSV-escaped (tabs/newlines encoded, 200-char cap).
    println!("id\tkind\tscore\twhy\tlabel");
    for h in &out.hits {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            escape_tsv_field(&h.node_id),
            escape_tsv_field(&h.kind),
            h.score,
            escape_tsv_field(&h.why),
            escape_tsv_field(&h.label)
        );
    }
    if !out.warnings.is_empty() {
        println!("warnings: {}", out.warnings.join(", "));
    }
}

fn print_report_json(report: &innen_core::doctor::Report) {
    println!(
        "{}",
        serde_json::to_string(report).expect("report serializes")
    );
}

fn print_report_human(report: &innen_core::doctor::Report) {
    // Bounded P1 human table: doctor checks (TSV-escaped, 200-char cap).
    println!("check\tok\tdetail");
    for c in &report.checks {
        println!(
            "{}\t{}\t{}",
            escape_tsv_field(&c.name),
            if c.ok { "ok" } else { "FAIL" },
            escape_tsv_field(&c.detail)
        );
    }
    println!("exit_code\t{}", report.exit_code);
}

fn cmd_query(root: &std::path::Path, format: &str, args: &QueryArgs) -> i32 {
    let params = innen_core::query::QueryParams {
        q: args.q.clone(),
        as_of: args.as_of.clone(),
        limit: args.limit,
        include_expired: args.include_expired,
    };
    match innen_core::query::query(root, &params) {
        Ok(out) => {
            if is_human(format) {
                print_query_human(&out);
            } else {
                print_query_json(&out);
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_graph_node(root: &std::path::Path, format: &str, args: &GraphNodeArgs) -> i32 {
    use serde_json::json;
    // Thin call: custom-provenance rule lives in innen-core::graph.
    let parsed: innen_core::graph::NodeType = args.kind.parse().unwrap();
    if let Err(e) = innen_core::graph::check_node_provenance(&parsed, args.provenance.as_deref()) {
        eprintln!("error: {e}");
        return 1;
    }
    let mut payload = json!({
        "id": args.id,
        "type": args.kind,
        "label": args.label,
    });
    if let Some(body) = &args.body {
        payload["body"] = json!(body);
    }
    if let Some(prov) = &args.provenance {
        payload["provenance"] = json!(prov);
    }
    let journal = match innen_core::journal::Journal::open(root) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match journal.append("node.upsert", &payload) {
        Ok(id) => {
            if is_human(format) {
                println!("created\t{}", escape_tsv_field(&id));
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({"id": id}))
                        .expect("node output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_graph_relate(root: &std::path::Path, format: &str, args: &GraphRelateArgs) -> i32 {
    use serde_json::json;
    let edge_ty: innen_core::graph::EdgeType = args.edge.parse().unwrap();
    // Thin call: endpoint-kind resolution + adjacency/provenance validation
    // lives in innen-core::graph (best-effort open may quarantine; that is the
    // normal write-path behavior for graph mutations).
    if let Err(e) = innen_core::graph::validate_relate_request(
        root,
        &args.from,
        &edge_ty,
        &args.to,
        args.provenance.as_deref(),
    ) {
        eprintln!("error: {e}");
        return 1;
    }
    let mut payload = json!({
        "from": args.from,
        "type": edge_ty.to_string(),
        "to": args.to,
    });
    if let Some(w) = args.weight {
        payload["weight"] = json!(w);
    }
    if let Some(vf) = &args.valid_from {
        payload["valid_from"] = json!(vf);
    }
    if let Some(vu) = &args.valid_until {
        payload["valid_until"] = json!(vu);
    }
    if let Some(prov) = &args.provenance {
        payload["provenance"] = json!(prov);
    }
    // Re-open is cheap; reuse open handle would self-block on the fs2 lock,
    // so open once more here for the append (sequential, not nested).
    let journal = match innen_core::journal::Journal::open(root) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match journal.append("edge.assert", &payload) {
        Ok(id) => {
            if is_human(format) {
                println!("created\t{}", escape_tsv_field(&id));
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({"id": id}))
                        .expect("relate output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_graph_retract(root: &std::path::Path, format: &str, args: &GraphRetractArgs) -> i32 {
    let edge_ty: innen_core::graph::EdgeType = args.edge.parse().unwrap();
    let payload = serde_json::json!({
        "from": args.from,
        "type": edge_ty.to_string(),
        "to": args.to,
    });
    let journal = match innen_core::journal::Journal::open(root) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match journal.append("edge.retract", &payload) {
        Ok(id) => {
            if is_human(format) {
                println!("retracted\t{}", escape_tsv_field(&id));
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({"id": id}))
                        .expect("retract output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_index_rebuild(root: &std::path::Path, format: &str) -> i32 {
    match innen_core::index::rebuild(root) {
        Ok(()) => {
            if is_human(format) {
                println!("index\trebuilt");
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({"ok": true}))
                        .expect("index output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_doctor(root: &std::path::Path, format: &str) -> i32 {
    let report = innen_core::doctor::run(root);
    if is_human(format) {
        print_report_human(&report);
    } else {
        print_report_json(&report);
    }
    report.exit_code as i32
}

fn cmd_lint(root: &std::path::Path, format: &str) -> i32 {
    let report = innen_core::doctor::lint(root);
    if is_human(format) {
        print_report_human(&report);
    } else {
        print_report_json(&report);
    }
    report.exit_code as i32
}

fn config_value(cfg: &innen_core::config::Config, key: &str) -> Option<String> {
    match key {
        "root" => Some(cfg.root.to_string_lossy().into_owned()),
        "format" => Some(cfg.format.clone()),
        "rebuild_on_open" => Some(cfg.rebuild_on_open.to_string()),
        _ => None,
    }
}

fn cmd_config_get(root: &std::path::Path, format: &str, key: &str) -> i32 {
    // Resolved config (env > machine.json > innen.toml > builtin). The global
    // --format rendering flag is intentionally NOT a config override here:
    // `config get format` reports the persisted value.
    let (cfg, warnings) =
        innen_core::config::load(root, &innen_core::config::CliOverrides::default());
    for w in &warnings {
        eprintln!("{w}");
    }
    let Some(value) = config_value(&cfg, key) else {
        eprintln!("error: unknown config key: {key} (root|format|rebuild_on_open)");
        return 1;
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&value));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": value}))
                .expect("config output serializes")
        );
    }
    0
}

fn cmd_config_set(root: &std::path::Path, format: &str, key: &str, value: &str) -> i32 {
    // Thin call: validation + machine.json persistence lives in innen-core::config.
    let echo = match innen_core::config::set_machine_value(root, key, value) {
        Ok(echo) => echo,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if is_human(format) {
        println!("{key} = {}", escape_tsv_field(&echo));
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": echo}))
                .expect("config output serializes")
        );
    }
    0
}

/// Resolve the `rclone` binary via PATH lookup (CLI-only; unit tests
/// inject the fixture path directly into `Rclone { bin }`).
fn resolve_rclone_bin() -> Result<PathBuf, String> {
    let path_var =
        std::env::var_os("PATH").ok_or_else(|| "rclone not found in PATH".to_string())?;
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let cand = dir.join("rclone");
        if cand.is_file() {
            return Ok(cand);
        }
    }
    Err("rclone not found in PATH".to_string())
}

fn cmd_guide(format: &str) -> i32 {
    let text = innen_core::parity::guide_text();
    if is_human(format) {
        println!("{text}");
    } else {
        println!(
            "{}",
            serde_json::to_string(&GuideJson {
                text: text.to_string(),
            })
            .expect("guide output serializes")
        );
    }
    0
}

fn cmd_search(root: &std::path::Path, format: &str, args: &SearchArgs) -> i32 {
    let hits = innen_core::parity::search(root, &args.keyword, args.limit);
    if is_human(format) {
        println!("node_id\texcerpt");
        for h in &hits {
            println!(
                "{}\t{}",
                escape_tsv_field(&h.node_id),
                escape_tsv_field(&h.excerpt)
            );
        }
    } else {
        let out = SearchJson {
            hits: hits
                .into_iter()
                .map(|h| SearchHitJson {
                    node_id: h.node_id,
                    excerpt: h.excerpt,
                })
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("search output serializes")
        );
    }
    0
}

fn cmd_status(root: &std::path::Path, format: &str) -> i32 {
    let s = innen_core::parity::status(root);
    if is_human(format) {
        println!("nodes\tedges\tevents\tartifacts");
        println!("{}\t{}\t{}\t{}", s.nodes, s.edges, s.events, s.artifacts);
    } else {
        let out = StatusJson {
            nodes: s.nodes,
            edges: s.edges,
            events: s.events,
            artifacts: s.artifacts,
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("status output serializes")
        );
    }
    0
}

fn cmd_timeline(root: &std::path::Path, format: &str, args: &TimelineArgs) -> i32 {
    let rows = innen_core::parity::timeline(root, args.filter.as_deref());
    if is_human(format) {
        println!("observed_utc\top\tsummary");
        for e in &rows {
            println!(
                "{}\t{}\t{}",
                escape_tsv_field(&e.observed_utc),
                escape_tsv_field(&e.op),
                escape_tsv_field(&e.summary)
            );
        }
    } else {
        let out = TimelineJson {
            entries: rows
                .into_iter()
                .map(|e| TimelineEntryJson {
                    observed_utc: e.observed_utc,
                    op: e.op,
                    summary: e.summary,
                })
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string(&out).expect("timeline output serializes")
        );
    }
    0
}

fn cmd_project(root: &std::path::Path, format: &str, id: &str) -> i32 {
    match innen_core::parity::project_render(root, id) {
        Ok(render) => {
            if is_human(format) {
                print!("{render}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&ProjectJson {
                        id: id.to_string(),
                        render,
                    })
                    .expect("project output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_profile(root: &std::path::Path, format: &str) -> i32 {
    match innen_core::parity::profile_render(root) {
        Ok(render) => {
            if is_human(format) {
                print!("{render}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&ProfileJson { render })
                        .expect("profile output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_artifact_add(root: &std::path::Path, format: &str, args: &ArtifactAddArgs) -> i32 {
    match innen_core::artifact::add(root, &args.file, args.project.as_deref()) {
        Ok(r) => {
            if is_human(format) {
                println!("sha256\tpath\tbytes");
                println!(
                    "{}\t{}\t{}",
                    escape_tsv_field(&r.sha256),
                    escape_tsv_field(&r.stored_path.to_string_lossy()),
                    r.bytes
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&ArtifactJson {
                        sha256: r.sha256,
                        path: r.stored_path.to_string_lossy().into_owned(),
                        bytes: r.bytes,
                    })
                    .expect("artifact output serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_cloud_status(format: &str, remote: &str) -> i32 {
    let bin = match resolve_rclone_bin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let r = innen_core::cloud::Rclone { bin };
    match r.status(remote) {
        Ok(output) => {
            if is_human(format) {
                print!("{output}");
                if !output.ends_with('\n') {
                    println!();
                }
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&CloudStatusJson {
                        remote: remote.to_string(),
                        output,
                    })
                    .expect("cloud status serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_cloud_doctor(format: &str) -> i32 {
    let bin = match resolve_rclone_bin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let r = innen_core::cloud::Rclone { bin };
    match r.doctor() {
        Ok(output) => {
            if is_human(format) {
                print!("{output}");
                if !output.ends_with('\n') {
                    println!();
                }
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&CloudDoctorJson { output })
                        .expect("cloud doctor serializes")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_completions(shell: &str) -> i32 {
    let clap_shell = match shell {
        "bash" => clap_complete::Shell::Bash,
        "zsh" => clap_complete::Shell::Zsh,
        "fish" => clap_complete::Shell::Fish,
        "powershell" => clap_complete::Shell::PowerShell,
        _ => {
            eprintln!("error: unknown shell: {shell}");
            return 1;
        }
    };
    let mut cmd = Cli::command();
    clap_complete::generate(clap_shell, &mut cmd, "innen", &mut std::io::stdout());
    0
}

fn main() {
    let cli = Cli::parse();
    let code = match &cli.command {
        Commands::Query(args) => {
            let root = resolve_root(cli.root.clone());
            cmd_query(&root, &cli.format, args)
        }
        Commands::Graph { op } => {
            let root = resolve_root(cli.root.clone());
            match op {
                GraphOp::Node(a) => cmd_graph_node(&root, &cli.format, a),
                GraphOp::Relate(a) => cmd_graph_relate(&root, &cli.format, a),
                GraphOp::Retract(a) => cmd_graph_retract(&root, &cli.format, a),
            }
        }
        Commands::Index { op } => {
            let root = resolve_root(cli.root.clone());
            match op {
                IndexOp::Rebuild => cmd_index_rebuild(&root, &cli.format),
            }
        }
        Commands::Doctor => {
            let root = resolve_root(cli.root.clone());
            cmd_doctor(&root, &cli.format)
        }
        Commands::Lint => {
            let root = resolve_root(cli.root.clone());
            cmd_lint(&root, &cli.format)
        }
        Commands::Config { op } => {
            let root = resolve_root(cli.root.clone());
            match op {
                ConfigOp::Get { key } => cmd_config_get(&root, &cli.format, key),
                ConfigOp::Set { key, value } => cmd_config_set(&root, &cli.format, key, value),
            }
        }
        Commands::Completions { shell } => cmd_completions(shell),
        Commands::Guide => cmd_guide(&cli.format),
        Commands::Search(args) => {
            let root = resolve_root(cli.root.clone());
            cmd_search(&root, &cli.format, args)
        }
        Commands::Status => {
            let root = resolve_root(cli.root.clone());
            cmd_status(&root, &cli.format)
        }
        Commands::Timeline(args) => {
            let root = resolve_root(cli.root.clone());
            cmd_timeline(&root, &cli.format, args)
        }
        Commands::Project(args) => {
            let root = resolve_root(cli.root.clone());
            cmd_project(&root, &cli.format, &args.id)
        }
        Commands::Profile => {
            let root = resolve_root(cli.root.clone());
            cmd_profile(&root, &cli.format)
        }
        Commands::Artifact { op } => {
            let root = resolve_root(cli.root.clone());
            match op {
                ArtifactOp::Add(a) => cmd_artifact_add(&root, &cli.format, a),
            }
        }
        Commands::Cloud { op } => match op {
            CloudOp::Status(a) => cmd_cloud_status(&cli.format, &a.remote),
            CloudOp::Doctor => cmd_cloud_doctor(&cli.format),
        },
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_escape_pins_tabs_newlines_and_cjk_truncation() {
        // Embedded tabs/newlines must not break TSV rows.
        assert_eq!(escape_tsv_field("a\tb\nc\rd"), "a\\tb\\nc\\rd");
        // Long fields truncate at 200 chars (chars, not bytes — CJK safe) with ….
        let long = "字".repeat(250);
        let got = escape_tsv_field(&long);
        assert_eq!(got, format!("{}…", "字".repeat(TSV_FIELD_LIMIT)));
        assert_eq!(got.chars().count(), TSV_FIELD_LIMIT + 1);
        // Short CJK passes through unchanged.
        assert_eq!(escape_tsv_field("臺北"), "臺北");
    }
}

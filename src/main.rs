//! P1 CLI dispatch (Task 8d).
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

use std::collections::BTreeMap;
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
    about = "innen P1 knowledge CLI",
    long_about = "innen P1 knowledge CLI.\n\nKB root resolution: --root <dir> > INNEN_ROOT env (non-empty) > cwd."
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

fn is_human(format: &str) -> bool {
    format == "human"
}

fn print_query_json(out: &innen_core::query::QueryOutput) {
    println!(
        "{}",
        serde_json::to_string(out).expect("query output serializes")
    );
}

fn print_query_human(out: &innen_core::query::QueryOutput) {
    // Bounded P1 human table: id/kind/score/why (+ label for readability).
    println!("id\tkind\tscore\twhy\tlabel");
    for h in &out.hits {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            h.node_id, h.kind, h.score, h.why, h.label
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
    // Bounded P1 human table: doctor checks.
    println!("check\tok\tdetail");
    for c in &report.checks {
        println!(
            "{}\t{}\t{}",
            c.name,
            if c.ok { "ok" } else { "FAIL" },
            c.detail
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

fn is_uri_shape(s: &str) -> bool {
    !s.is_empty() && (s.starts_with('/') || s.contains("://"))
}

fn cmd_graph_node(root: &std::path::Path, format: &str, args: &GraphNodeArgs) -> i32 {
    use serde_json::json;
    // Custom kinds require provenance (mirror graph::validate open-world rule).
    let parsed: innen_core::graph::NodeType = args.kind.parse().unwrap();
    if matches!(parsed, innen_core::graph::NodeType::Custom(_))
        && args
            .provenance
            .as_deref()
            .is_none_or(|p| p.trim().is_empty())
    {
        eprintln!("error: custom node kind requires non-empty --provenance");
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
                println!("created\t{id}");
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
    // Resolve endpoint kinds from current materialized state for validation.
    // Missing nodes skip validation (journal permits dangling; doctor reports).
    let mut from_ty_opt: Option<innen_core::graph::NodeType> = None;
    let mut to_ep_opt: Option<innen_core::graph::Endpoint> = None;
    // Best-effort read of current nodes (open may quarantine; that is the
    // normal write-path behavior for graph mutations).
    if let Ok(journal) = innen_core::journal::Journal::open(root) {
        if let Ok(entries) = journal.read_all() {
            let events: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    json!({
                        "op": e.op,
                        "payload": e.payload,
                        "observed_utc": e.observed_utc,
                    })
                })
                .collect();
            let m = innen_core::graph::materialize(&events, None, true);
            if let Some(node) = m.nodes.get(&args.from) {
                let ty = node
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Custom");
                from_ty_opt = Some(ty.parse().unwrap());
            }
            if is_uri_shape(&args.to) {
                to_ep_opt = Some(innen_core::graph::Endpoint::Uri(args.to.clone()));
            } else if let Some(node) = m.nodes.get(&args.to) {
                let ty = node
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Custom");
                let nt: innen_core::graph::NodeType = ty.parse().unwrap();
                to_ep_opt = Some(innen_core::graph::Endpoint::Node(nt));
            }
        }
    }
    if let (Some(from_ty), Some(to_ep)) = (from_ty_opt, to_ep_opt) {
        if let Err(e) =
            innen_core::graph::validate(&from_ty, &edge_ty, &to_ep, args.provenance.as_deref())
        {
            eprintln!("error: {e}");
            return 1;
        }
    } else {
        // No stored kinds to check against: still enforce the custom-party
        // provenance rule directly on the declared edge string.
        let edge_is_custom = matches!(edge_ty, innen_core::graph::EdgeType::Custom(_));
        let to_is_uri = is_uri_shape(&args.to);
        if edge_is_custom || to_is_uri {
            // URI legality still applies even without stored nodes.
            if to_is_uri
                && !matches!(
                    edge_ty,
                    innen_core::graph::EdgeType::LocatedAt
                        | innen_core::graph::EdgeType::OriginatedAt
                        | innen_core::graph::EdgeType::Custom(_)
                )
            {
                eprintln!("error: illegal adjacency: ? -[{}]-> {}", edge_ty, args.to);
                return 1;
            }
            if edge_is_custom
                && args
                    .provenance
                    .as_deref()
                    .is_none_or(|p| p.trim().is_empty())
            {
                eprintln!("error: custom node/edge requires non-empty provenance");
                return 1;
            }
        }
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
                println!("created\t{id}");
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
                println!("retracted\t{id}");
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
        println!("{key} = {value}");
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
    if !matches!(key, "root" | "format" | "rebuild_on_open") {
        eprintln!("error: unknown config key: {key} (root|format|rebuild_on_open)");
        return 1;
    }
    if key == "rebuild_on_open"
        && !matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "false")
    {
        eprintln!("error: rebuild_on_open must be true|false");
        return 1;
    }
    if (key == "root" || key == "format") && value.trim().is_empty() {
        eprintln!("error: {key} must be non-empty");
        return 1;
    }
    let stored = if key == "rebuild_on_open" {
        let b = value.trim().eq_ignore_ascii_case("true");
        serde_json::Value::Bool(b)
    } else {
        serde_json::Value::String(value.to_string())
    };
    let machine_path = root.join(".innen").join("machine.json");
    if let Err(e) = std::fs::create_dir_all(root.join(".innen")) {
        eprintln!("error: {e}");
        return 1;
    }
    let mut map: BTreeMap<String, serde_json::Value> = match std::fs::read(&machine_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    map.insert(key.to_string(), stored);
    let text = serde_json::to_string(&map).expect("machine.json serializes");
    if let Err(e) = std::fs::write(&machine_path, format!("{text}\n")) {
        eprintln!("error: {e}");
        return 1;
    }
    // Echo the resolved stored value (sorted-keys deterministic).
    let echo = match key {
        "rebuild_on_open" => value.trim().to_ascii_lowercase(),
        _ => value.to_string(),
    };
    if is_human(format) {
        println!("{key} = {echo}");
    } else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"key": key, "value": echo}))
                .expect("config output serializes")
        );
    }
    0
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
    };
    std::process::exit(code);
}

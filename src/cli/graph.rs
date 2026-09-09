#[derive(clap::Subcommand)]
pub(super) enum GraphOp {
    /// Append a node.upsert event.
    Node(GraphNodeArgs),
    /// Append an edge.assert event (adjacency-validated).
    Relate(GraphRelateArgs),
    /// Append an edge.retract event.
    Retract(GraphRetractArgs),
}

#[derive(clap::Args)]
pub(super) struct GraphNodeArgs {
    /// Node id.
    #[arg(long)]
    pub(super) id: String,
    /// Node kind (NodeType display name; unknown stays Custom).
    #[arg(long, visible_alias = "type")]
    pub(super) kind: String,
    /// Human label.
    #[arg(long)]
    pub(super) label: String,
    /// Optional body text.
    #[arg(long)]
    pub(super) body: Option<String>,
    /// Provenance (required when kind is custom).
    #[arg(long)]
    pub(super) provenance: Option<String>,
}

#[derive(clap::Args)]
pub(super) struct GraphRelateArgs {
    /// From node id.
    #[arg(long)]
    pub(super) from: String,
    /// Edge type (SCREAMING_SNAKE_CASE; unknown stays Custom).
    #[arg(long, visible_alias = "type")]
    pub(super) edge: String,
    /// To node id or URI (`/abs` or `scheme://...` only under LOCATED_AT/ORIGINATED_AT).
    #[arg(long)]
    pub(super) to: String,
    /// Optional weight.
    #[arg(long)]
    pub(super) weight: Option<f32>,
    /// Optional valid-from `YYYY-MM-DDTHH:MM:SSZ`.
    #[arg(long = "valid-from")]
    pub(super) valid_from: Option<String>,
    /// Optional valid-until `YYYY-MM-DDTHH:MM:SSZ`.
    #[arg(long = "valid-until")]
    pub(super) valid_until: Option<String>,
    /// Provenance (required for custom parties).
    #[arg(long)]
    pub(super) provenance: Option<String>,
    /// Permit missing node endpoints for migration or out-of-order ingestion.
    #[arg(long)]
    pub(super) allow_dangling: bool,
}

#[derive(clap::Args)]
pub(super) struct GraphRetractArgs {
    /// From node id.
    #[arg(long)]
    pub(super) from: String,
    /// Edge type.
    #[arg(long, visible_alias = "type")]
    pub(super) edge: String,
    /// To node id.
    #[arg(long)]
    pub(super) to: String,
}

use super::util::{escape_tsv_field, is_human};

pub(super) fn cmd_graph_retract(
    root: &std::path::Path,
    format: &str,
    args: &GraphRetractArgs,
) -> i32 {
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

pub(super) fn cmd_graph_node(root: &std::path::Path, format: &str, args: &GraphNodeArgs) -> i32 {
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

pub(super) fn cmd_graph_relate(
    root: &std::path::Path,
    format: &str,
    args: &GraphRelateArgs,
) -> i32 {
    use innen_core::edge_write::{append_edge_assert, EdgeAssert};
    let edge_ty: innen_core::graph::EdgeType = args.edge.parse().unwrap();
    // Single choke point: validate-before-open + append live in
    // innen-core::edge_write, shared by every edge writer. Validating the
    // raw file first is load-bearing: opening would quarantine corrupt
    // lines and rewrite the journal before validation sees them.
    let req = EdgeAssert {
        from: &args.from,
        edge: edge_ty,
        to: &args.to,
        weight: args.weight,
        valid_from: args.valid_from.as_deref(),
        valid_until: args.valid_until.as_deref(),
        provenance: args.provenance.as_deref(),
        allow_dangling: args.allow_dangling,
    };
    match append_edge_assert(root, &req) {
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

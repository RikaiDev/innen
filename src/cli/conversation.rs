use super::util::is_human;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct ConversationArgs {
    /// UUID or native session ID (for example OpenCode ses_...).
    #[arg(required_unless_present_any = ["decode_packet", "validate_packet", "encode_json"])]
    uuid: Option<String>,
    /// Validate and restore a saved self-describing conversation packet, offline.
    #[arg(long, conflicts_with_all = ["uuid", "validate_packet", "encode_json"])]
    decode_packet: Option<PathBuf>,
    /// Validate a saved packet and emit it unchanged for model input, offline.
    #[arg(long, conflicts_with_all = ["uuid", "decode_packet", "encode_json"])]
    validate_packet: Option<PathBuf>,
    /// Encode a saved ordinary conversation JSON page, offline.
    #[arg(long, conflicts_with_all = ["uuid", "decode_packet", "validate_packet"])]
    encode_json: Option<PathBuf>,
    /// Opt-in reference-token-selected conversation grammar; legacy remains default.
    #[arg(long, default_value = "legacy", value_parser = ["legacy", "conversation"])]
    codec: String,
    /// Source store directory (or OpenCode database file). Requires --source.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Tool to read; auto searches standard local stores and rejects ambiguity.
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Dialogue is text; events retains native fields; index is incomplete navigation previews.
    #[arg(long, default_value = "dialogue", value_parser = ["dialogue", "events", "index", "context"])]
    view: String,
    /// Factor repeated event fields without summarizing. Falls back if bytes grow.
    #[arg(long)]
    compact: bool,
    /// Experimentally share exact string prefixes/suffixes; no semantic edits.
    #[arg(long, requires = "compact")]
    deltas: bool,
    /// Reference image data URIs and known encrypted fields. Requires events view.
    #[arg(long, conflicts_with = "attachment")]
    attachment_refs: bool,
    /// Retrieve a source string at this JSON pointer; uses events view and limit 1.
    #[arg(long, requires = "expect_sha256", conflicts_with = "compact")]
    attachment: Option<String>,
    /// Expected UTF-8 string hash from an attachment reference; fail on changes.
    #[arg(long, requires = "attachment")]
    expect_sha256: Option<String>,
    /// Batch exact one-based source lines (comma-separated); returns raw events in source order.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["offset", "limit", "attachment"])]
    lines: Vec<usize>,
    /// Zero-based physical JSONL row; use next_offset from the previous page.
    #[arg(long, default_value_t = 0)]
    offset: usize,
    /// Maximum matching events per page (1..100).
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=100))]
    limit: u16,
}

pub(super) fn cmd_conversation(format: &str, args: &ConversationArgs) -> i32 {
    if let Some(path) = args
        .decode_packet
        .as_ref()
        .or(args.validate_packet.as_ref())
        .or(args.encode_json.as_ref())
    {
        let result = (|| -> Result<serde_json::Value, String> {
            if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
                return Err("packet file exceeds 16 MiB admission bound".into());
            }
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if args.encode_json.is_some() {
                if !value["records"].is_array() || value.get("encoding").is_some() {
                    return Err(
                        "encode-json requires an ordinary conversation page with records".into(),
                    );
                }
                innen_core::conversation::grammar::encode(&value, &value)
            } else {
                let decoded = innen_core::conversation::grammar::decode(&value)?;
                Ok(if args.validate_packet.is_some() {
                    value
                } else {
                    decoded
                })
            }
        })();
        return match result {
            Ok(value) => {
                println!(
                    "{}",
                    if is_human(format) {
                        serde_json::to_string_pretty(&value).unwrap()
                    } else {
                        value.to_string()
                    }
                );
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }
    let Some(uuid) = args.uuid.as_deref() else {
        eprintln!("error: missing conversation ID");
        return 1;
    };
    if args.codec == "conversation" && (args.attachment_refs || args.attachment.is_some()) {
        eprintln!("error: conversation grammar requires complete page data, not attachment externalization");
        return 1;
    }
    let result = if !args.lines.is_empty() {
        innen_core::conversation::read_lines(
            args.source_root.as_deref(),
            &args.source,
            uuid,
            &args.lines,
        )
    } else {
        innen_core::conversation::read(
            args.source_root.as_deref(),
            &args.source,
            uuid,
            if args.attachment.is_some() {
                "events"
            } else {
                &args.view
            },
            args.offset,
            if args.attachment.is_some() {
                1
            } else {
                usize::from(args.limit)
            },
        )
    };
    match result {
        Ok(page) => {
            let original = serde_json::to_value(&page).expect("page serializes");
            match innen_core::conversation::format_page(
                page,
                args.compact,
                args.deltas,
                args.attachment_refs,
                args.attachment.as_deref(),
                args.expect_sha256.as_deref(),
                args.offset,
            ) {
                Ok(formatted) => {
                    let formatted = if args.codec == "conversation" {
                        match innen_core::conversation::grammar::encode(&original, &formatted) {
                            Ok(value) => value,
                            Err(e) => {
                                eprintln!("error: {e}");
                                return 1;
                            }
                        }
                    } else {
                        formatted
                    };
                    let output = if is_human(format) {
                        serde_json::to_string_pretty(&formatted)
                    } else {
                        serde_json::to_string(&formatted)
                    };
                    println!("{}", output.expect("formatted page serializes"));
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

use super::util::is_human;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct CompressArgs {
    /// UUID or native session ID (for example OpenCode ses_...).
    #[arg(required = true)]
    uuid: String,
    /// Source store directory (or OpenCode database file). Requires --source.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Tool to read; auto searches standard local stores and rejects ambiguity.
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "claude", "codex", "copilot", "cursor", "freebuff", "gemini", "grok", "kilo", "kiro", "mcode", "opencode", "qwen", "vscode"])]
    source: String,
    /// Project path (relative to KB root) for project field in compression output.
    #[arg(long)]
    project: Option<String>,
    /// Output format for compressed conversation.
    #[arg(long, default_value = "json", value_parser = ["json", "human"])]
    output: String,
}

pub(super) fn cmd_compress(format: &str, args: &CompressArgs) -> i32 {
    let is_human = is_human(format);
    let project = args.project.clone().unwrap_or_else(|| ".".to_string());

    // Read full transcript for compression
    let (source, records) = match innen_core::conversation::read_all_dialogue_with_source(
        args.source_root.as_deref(),
        &args.source,
        &args.uuid,
    ) {
        Ok((src, recs)) => (src, recs),
        Err(e) => {
            eprintln!("error: failed to read conversation: {e}");
            return 1;
        }
    };

    if records.is_empty() {
        eprintln!("error: no records found for session");
        return 1;
    }

    // Compute source hash and bytes from the records
    let source_bytes = records.iter().map(|r| r.line as u64).sum::<u64>();
    let source_content = serde_json::to_string(&records).unwrap_or_default();
    let source_sha256 = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(source_content.as_bytes());
        let result = hasher.finalize();
        let mut hex_str = String::with_capacity(64);
        for byte in result {
            use std::fmt::Write;
            write!(&mut hex_str, "{:02x}", byte).unwrap();
        }
        hex_str
    };

    // Compress the conversation
    let compress_result = innen_core::conversation::compress::compress_conversation(
        &args.uuid,
        source,
        &project,
        &records,
        &source_sha256,
        source_bytes,
        true, // session_closed - assume closed for now
        None, // latest_checkpoint - not available in this context
    );

    let compressed = match compress_result {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: compression failed: {e}");
            return 1;
        }
    };

    // Output the compressed conversation
    let output = if args.output == "json" {
        if is_human {
            serde_json::to_string_pretty(&compressed).unwrap()
        } else {
            serde_json::to_string(&compressed).unwrap()
        }
    } else {
        // Human-readable summary
        format!(
            "Compressed Conversation: {}\n\
             Session: {}\n\
             Source: {:?}\n\
             Project: {}\n\
             Records: {}\n\
             Decisions: {}\n\
             Code Changes: {}\n\
             Blockers: {}\n\
             Handoff Instructions: {}\n\
             User Requirements: {}\n\
             Deliverables: {}\n\
             Unfinished Score: {:.2}\n\
             Handoff Priority: {:.2}\n\
             Knowledge Nodes: {}\n\
             Session Closed: {}\n",
            compressed.conversation_id,
            compressed.session_id,
            compressed.source,
            compressed.project,
            compressed.total_records,
            compressed.extraction.decisions.len(),
            compressed.extraction.code_changes.len(),
            compressed.extraction.blockers.len(),
            compressed.extraction.handoff_instructions.len(),
            compressed.extraction.user_requirements.len(),
            compressed.extraction.deliverables.len(),
            compressed.extraction.unfinished_score,
            compressed.extraction.handoff_priority,
            compressed.knowledge_node_ids.len(),
            compressed.session_closed
        )
    };

    println!("{}", output);
    0
}

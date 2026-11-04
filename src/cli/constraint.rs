use super::util::{escape_tsv_field, is_human};
use std::io::Read;
use std::path::PathBuf;

#[derive(clap::Subcommand)]
pub(super) enum ConstraintOp {
    /// Record a requirement the user stated, owned by a project (node + edge in one write).
    ///
    /// Shell-safe input: put the words in a file, or pipe them with `--text-file -`
    /// (zsh: `print -r -- 'text' | innen constraint add ... --text-file -`), so
    /// `!` `?` `*` `(` need no quoting on the command line.
    Add(ConstraintAddArgs),
    /// List the user's own turns of a conversation, with source lines, to choose requirements from.
    Extract(ConstraintExtractArgs),
}

#[derive(clap::Args)]
pub(super) struct ConstraintExtractArgs {
    /// Conversation UUID or native session id.
    session: String,
    /// Tool to read; auto searches the standard local stores.
    #[arg(long, default_value = "auto")]
    tool: String,
    /// Keep only turns that contain an instruction, correction or complaint cue.
    #[arg(long)]
    directives: bool,
    /// Source store directory; requires --tool.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Longest text shown per turn, in characters; the full text stays in the source.
    #[arg(long, default_value_t = 600)]
    max_chars: usize,
}

#[derive(clap::Args)]
pub(super) struct ConstraintAddArgs {
    /// Project node id, for example `project:ntuh-yunlin`.
    #[arg(long)]
    project: String,
    /// Where the words can be checked, for example `agy:<session>#user-turn-30`.
    #[arg(long, required_unless_present = "session", conflicts_with = "session")]
    source: Option<String>,
    /// Record turns of this conversation verbatim; the source is derived and the text checked.
    #[arg(long, requires = "line")]
    session: Option<String>,
    /// One-based source lines of the user turns to record (comma-separated).
    #[arg(long, value_delimiter = ',', requires = "session")]
    line: Vec<usize>,
    /// Tool to read with --session; auto searches the standard local stores.
    #[arg(long, default_value = "auto", requires = "session")]
    tool: String,
    /// Source store directory for --session; requires --tool.
    #[arg(long, requires = "session")]
    source_root: Option<PathBuf>,
    /// Record only this passage; it must occur word for word in the one --line.
    #[arg(long, requires = "session")]
    quote: Option<String>,
    /// The user's words, verbatim.
    #[arg(long, conflicts_with_all = ["text_file", "session"])]
    text: Option<String>,
    /// Read the words from this file; `-` reads standard input.
    #[arg(long, conflicts_with = "session")]
    text_file: Option<PathBuf>,
    /// Node id; derived from the text when omitted.
    #[arg(long)]
    id: Option<String>,
    /// Short label; the first 40 characters of the text when omitted.
    #[arg(long)]
    label: Option<String>,
}

fn read_text(args: &ConstraintAddArgs) -> Result<String, String> {
    match (&args.text, &args.text_file) {
        (Some(t), _) => Ok(t.clone()),
        (None, Some(p)) if p.as_os_str() == "-" => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| e.to_string())?;
            Ok(buf)
        }
        (None, Some(p)) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display())),
        (None, None) => Err("--text or --text-file is required".into()),
    }
}

fn print_ids(format: &str, ids: &[String]) {
    if is_human(format) {
        for id in ids {
            println!("created\t{}", escape_tsv_field(id));
        }
    } else {
        println!("{}", serde_json::json!({"id": ids.first(), "ids": ids}));
    }
}

fn fail(e: &str) -> i32 {
    eprintln!("error: {e}");
    1
}

pub(super) fn cmd_constraint_add(
    root: &std::path::Path,
    format: &str,
    args: &ConstraintAddArgs,
) -> i32 {
    let result = if let Some(session) = &args.session {
        innen_core::constraint::add_from_session(
            root,
            args.source_root.as_deref(),
            &args.tool,
            &args.project,
            session,
            &args.line,
            args.quote.as_deref(),
        )
    } else {
        read_text(args).and_then(|text| {
            innen_core::constraint::add_constraint(
                root,
                &args.project,
                args.source.as_deref().unwrap_or_default(),
                &text,
                args.id.as_deref(),
                args.label.as_deref(),
            )
            .map(|id| vec![id])
        })
    };
    match result {
        Ok(ids) => {
            print_ids(format, &ids);
            0
        }
        Err(e) => fail(&e),
    }
}

pub(super) fn cmd_constraint_extract(format: &str, args: &ConstraintExtractArgs) -> i32 {
    let found = match innen_core::constraint_turns::user_turns(
        args.source_root.as_deref(),
        &args.tool,
        &args.session,
    ) {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    let total = found.turns.len();
    let rows: Vec<serde_json::Value> = found
        .turns
        .iter()
        .filter(|t| !args.directives || innen_core::constraint_turns::has_directive_cue(&t.text))
        .map(|t| {
            let cut = t.text.chars().count() > args.max_chars;
            let text: String = t.text.chars().take(args.max_chars).collect();
            serde_json::json!({"line": t.line, "text": text, "truncated": cut})
        })
        .collect();
    if is_human(format) {
        for r in &rows {
            println!(
                "L{}\t{}",
                r["line"],
                escape_tsv_field(r["text"].as_str().unwrap_or(""))
            );
        }
    } else {
        println!(
            "{}",
            serde_json::json!({
                "tool": found.tool,
                "session": found.session,
                "user_turns": total,
                "shown": rows.len(),
                "turns": rows,
            })
        );
    }
    0
}

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
}

#[derive(clap::Args)]
pub(super) struct ConstraintAddArgs {
    /// Project node id, for example `project:ntuh-yunlin`.
    #[arg(long)]
    project: String,
    /// Where the words can be checked, for example `agy:<session>#user-turn-30`.
    #[arg(long)]
    source: String,
    /// The user's words, verbatim.
    #[arg(
        long,
        required_unless_present = "text_file",
        conflicts_with = "text_file"
    )]
    text: Option<String>,
    /// Read the words from this file; `-` reads standard input.
    #[arg(long)]
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

pub(super) fn cmd_constraint(root: &std::path::Path, format: &str, op: &ConstraintOp) -> i32 {
    let ConstraintOp::Add(args) = op;
    let result = read_text(args).and_then(|text| {
        innen_core::constraint::add_constraint(
            root,
            &args.project,
            &args.source,
            &text,
            args.id.as_deref(),
            args.label.as_deref(),
        )
    });
    match result {
        Ok(id) => {
            if is_human(format) {
                println!("created\t{}", escape_tsv_field(&id));
            } else {
                println!("{}", serde_json::json!({"id": id}));
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

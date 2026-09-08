use super::util::is_human;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct ResumeArgs {
    /// UUID or native session ID. If omitted, inspects project metadata for an unambiguous candidate.
    uuid: Option<String>,
    /// Project root directory. Defaults to the current working directory.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Source store directory (or OpenCode database file). Requires --source.
    #[arg(long)]
    source_root: Option<PathBuf>,
    /// Tool to read; auto searches standard local stores and rejects ambiguity.
    #[arg(long, default_value = "auto", value_parser = ["auto", "agy", "antigravity", "codex", "claude", "gemini", "opencode", "grok", "copilot", "cursor", "vscode", "qwen"])]
    source: String,
    /// Projection view: brief (default), context, dialogue, events, index.
    #[arg(long, value_parser = ["brief", "dialogue", "events", "index", "context"])]
    view: Option<String>,
    /// Omit compact factoring (compact is enabled by default for resume).
    #[arg(long)]
    no_compact: bool,
    /// Omit delta prefix/suffix sharing (deltas is enabled by default for resume).
    #[arg(long)]
    no_deltas: bool,
    /// Reference image data URIs and known encrypted fields. Requires events view.
    #[arg(long, conflicts_with = "attachment")]
    attachment_refs: bool,
    /// Retrieve a source string at this JSON pointer; uses events view and limit 1.
    #[arg(long, requires = "expect_sha256")]
    attachment: Option<String>,
    /// Expected UTF-8 string hash from an attachment reference; fail on changes.
    #[arg(long, requires = "attachment")]
    expect_sha256: Option<String>,
    /// Batch exact one-based source lines (comma-separated); returns raw events in source order.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["offset", "limit", "attachment"])]
    lines: Vec<usize>,
    /// Zero-based physical JSONL row; use next_offset from the previous page.
    #[arg(long)]
    offset: Option<usize>,
    /// Maximum matching events per page (1..100).
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100))]
    limit: Option<u16>,
}

pub(super) fn cmd_resume(format: &str, args: &ResumeArgs) -> i32 {
    let (uuid, source_override) = match &args.uuid {
        Some(id) => (id.clone(), None),
        None => {
            let project_dir = match &args.project {
                Some(p) => p.clone(),
                None => match std::env::current_dir() {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("error: failed to determine current working directory: {e}");
                        return 1;
                    }
                },
            };
            match innen_core::conversation::resume::resolve_resume_target(
                &project_dir,
                &args.source,
                args.source_root.as_deref(),
            ) {
                Ok(innen_core::conversation::resume::ResumeTarget::Unambiguous(c)) => {
                    (c.id, Some(c.source.as_str().to_string()))
                }
                Ok(innen_core::conversation::resume::ResumeTarget::Ambiguous {
                    project,
                    candidates,
                }) => {
                    let report = serde_json::json!({
                        "status": "ambiguous",
                        "project": project,
                        "count": candidates.len(),
                        "candidates": candidates,
                        "message": "multiple candidate sessions found for project; specify session ID with: innen resume <session-id>"
                    });
                    if is_human(format) {
                        eprintln!(
                            "Multiple candidate sessions found for project {}:",
                            project.display()
                        );
                        for c in &candidates {
                            eprintln!(
                                "  - {} ({}, modified: {})",
                                c.id,
                                c.source.as_str(),
                                c.modified.as_deref().unwrap_or("unknown")
                            );
                        }
                        eprintln!("Specify session ID with: innen resume <session-id>");
                    } else {
                        println!("{}", serde_json::to_string(&report).unwrap());
                    }
                    return 1;
                }
                Ok(innen_core::conversation::resume::ResumeTarget::NotFound { project }) => {
                    let report = serde_json::json!({
                        "status": "not_found",
                        "project": project,
                        "message": format!(
                            "no candidate sessions found for project {}; specify session ID with: innen resume <session-id>",
                            project.display()
                        )
                    });
                    if is_human(format) {
                        eprintln!(
                            "No candidate sessions found for project {}.",
                            project.display()
                        );
                        eprintln!("Specify session ID with: innen resume <session-id>");
                    } else {
                        println!("{}", serde_json::to_string(&report).unwrap());
                    }
                    return 1;
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    return 1;
                }
            }
        }
    };

    let source = source_override.as_deref().unwrap_or(&args.source);
    let view = args.view.as_deref().unwrap_or("brief");
    let offset = args.offset.unwrap_or(0);
    let limit = usize::from(args.limit.unwrap_or(20));
    let mut brief_fallback_warning = None;
    if view == "brief"
        && args.lines.is_empty()
        && args.attachment.is_none()
        && args.offset.is_none()
        && args.limit.is_none()
    {
        match innen_core::conversation::brief(args.source_root.as_deref(), source, &uuid) {
            Ok(mut brief) => {
                if !brief.supported {
                    if args.view.is_some() {
                        eprintln!("error: --view brief is unavailable for this source; use --view context or --view events");
                        return 1;
                    }
                    // The default remains useful for non-Codex adapters: use
                    // the established bounded context reader and state why.
                    brief_fallback_warning = Some("brief unavailable for this source adapter; returned bounded context projection".to_string());
                } else {
                    let checkpoint_project = args
                        .project
                        .clone()
                        .or_else(|| brief.project.clone().map(std::path::PathBuf::from));
                    if let Some(project) = checkpoint_project.as_deref() {
                        match innen_core::conversation::checkpoint::latest_checkpoint_for_session(
                            project, &uuid,
                        ) {
                            Ok(checkpoint) => brief.checkpoint = checkpoint,
                            Err(error) => {
                                eprintln!("error: {error}");
                                return 1;
                            }
                        }
                    }
                    let value = serde_json::to_value(&brief).expect("resume brief serializes");
                    if is_human(format) {
                        println!("{}", serde_json::to_string_pretty(&value).unwrap());
                    } else {
                        println!("{}", serde_json::to_string(&value).unwrap());
                    }
                    return 0;
                }
            }
            Err(error) => {
                eprintln!("error: {error}");
                return 1;
            }
        }
    }
    let compact = !args.no_compact;
    let deltas = !args.no_deltas;

    let result = if !args.lines.is_empty() {
        innen_core::conversation::read_lines(
            args.source_root.as_deref(),
            source,
            &uuid,
            &args.lines,
        )
    } else {
        innen_core::conversation::read(
            args.source_root.as_deref(),
            source,
            &uuid,
            if args.attachment.is_some() {
                "events"
            } else if view == "brief" {
                "context"
            } else {
                view
            },
            offset,
            if args.attachment.is_some() { 1 } else { limit },
        )
    };

    match result {
        Ok(mut page) => {
            if let Some(warning) = brief_fallback_warning {
                page.warnings.push(warning);
            }
            match innen_core::conversation::format_page(
                page,
                compact,
                deltas,
                args.attachment_refs,
                args.attachment.as_deref(),
                args.expect_sha256.as_deref(),
                offset,
            ) {
                Ok(formatted) => {
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

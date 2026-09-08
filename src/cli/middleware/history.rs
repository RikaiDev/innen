use super::args::{MiddlewareArgs, MiddlewareOp};
use super::generic::print_middleware_unknown;
use super::input::read_middleware_request;
use crate::cli::util::is_human;
use std::path::PathBuf;
pub(in crate::cli) fn cmd_middleware(format: &str, args: &MiddlewareArgs) -> i32 {
    match &args.op {
        MiddlewareOp::HistoryHook {
            file,
            store,
            project,
            task,
            cwd,
        } => {
            // Hooks add context only for the explicitly bound workspace. They
            // never overwrite native history, trigger a model, or block a turn.
            let result = (|| -> Result<serde_json::Value, String> {
                if file.as_deref() == Some("-") {
                    return Err(
                        "history hook requires a file; stdin carries the native event".into(),
                    );
                }
                let event: serde_json::Value = read_middleware_request("-")?;
                if event["hook_event_name"] != "SessionStart" {
                    return Ok(serde_json::json!({}));
                }
                if !matches!(
                    event["source"].as_str(),
                    Some("startup" | "resume" | "clear" | "compact")
                ) {
                    return Ok(serde_json::json!({}));
                }
                let actual = event["cwd"].as_str().ok_or("hook workspace missing")?;
                let expected = std::fs::canonicalize(cwd).map_err(|e| e.to_string())?;
                if std::fs::canonicalize(actual).map_err(|e| e.to_string())? != expected {
                    return Ok(serde_json::json!({}));
                }
                if event["session_id"]
                    .as_str()
                    .is_none_or(|id| id.trim().is_empty())
                {
                    return Err("hook session identity missing".into());
                }
                let (request, history_file) = match (
                    file.as_deref(),
                    store.as_deref(),
                    project.as_deref(),
                    task.as_deref(),
                ) {
                    (Some(file), None, None, None) => (
                        read_middleware_request::<innen_core::middleware::history::Request>(file)?,
                        PathBuf::from(file),
                    ),
                    (None, Some(store), Some(project), Some(task)) => {
                        innen_core::middleware::history::store::load_with_snapshot(
                            store, project, task,
                        )
                        .map_err(|e| e.to_string())?
                    }
                    _ => return Err(
                        "history hook requires exactly --file or --store with --project and --task"
                            .into(),
                    ),
                };
                let output =
                    innen_core::middleware::history::prepare(request).map_err(|e| e.to_string())?;
                let context = serde_json::json!({
                    "resolution": output.resolution,
                    "history_source_sha256": output.history_source_sha256,
                    "history_file": history_file,
                    "packet": output.prepared.packet,
                });
                Ok(
                    serde_json::json!({"hookSpecificOutput":{"hookEventName":"SessionStart",
                    "additionalContext":serde_json::to_string(&context).expect("history hook context serializes")}}),
                )
            })();
            match result {
                Ok(value) => println!("{value}"),
                Err(reason) => println!(
                    "{}",
                    serde_json::json!({"systemMessage":format!("innen history context unavailable: {reason}")})
                ),
            }
            0
        }
        MiddlewareOp::HistoryPrepare { file, packet_only } => {
            let result = read_middleware_request::<innen_core::middleware::history::Request>(file)
                .and_then(|request| {
                    innen_core::middleware::history::prepare(request).map_err(|e| e.to_string())
                });
            match result {
                Ok(output) => {
                    if output.resolution != "prepared" {
                        let receipt =
                            serde_json::to_string(&output).expect("history receipt serializes");
                        if *packet_only {
                            eprintln!("{receipt}");
                        } else {
                            println!("{receipt}");
                        }
                        return 2;
                    }
                    if *packet_only {
                        println!("{}", output.prepared.packet_json);
                    } else {
                        println!(
                            "{}",
                            serde_json::to_string(&output).expect("history receipt serializes")
                        );
                    }
                    0
                }
                Err(reason) => {
                    let output = serde_json::json!({"resolution":"unknown","reason":reason});
                    if *packet_only {
                        eprintln!("{output}");
                    } else {
                        println!("{output}");
                    }
                    2
                }
            }
        }
        MiddlewareOp::HistoryExpand {
            file,
            expect_source_sha256,
            ids,
        } => {
            let result = read_middleware_request::<innen_core::middleware::history::Request>(file)
                .and_then(|request| {
                    innen_core::middleware::history::expand(request, expect_source_sha256, ids)
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(output) => {
                    println!("{output}");
                    0
                }
                Err(reason) => {
                    println!(
                        "{}",
                        serde_json::json!({"resolution":"unknown","reason":reason})
                    );
                    2
                }
            }
        }
        MiddlewareOp::HistorySave { file, store } => {
            let result = read_middleware_request::<innen_core::middleware::history::Request>(file)
                .and_then(|request| {
                    innen_core::middleware::history::store::save(store, request)
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(output) => {
                    println!(
                        "{}",
                        serde_json::to_string(&output).expect("history save receipt serializes")
                    );
                    0
                }
                Err(reason) => {
                    println!(
                        "{}",
                        serde_json::json!({"resolution":"unknown","reason":reason})
                    );
                    2
                }
            }
        }
        MiddlewareOp::HistoryLoad {
            store,
            project,
            task,
        } => match innen_core::middleware::history::store::load(store, project, task) {
            Ok(request) => {
                println!(
                    "{}",
                    serde_json::to_string(&request).expect("history request serializes")
                );
                0
            }
            Err(reason) => {
                println!(
                    "{}",
                    serde_json::json!({"resolution":"unknown","reason":reason.to_string()})
                );
                2
            }
        },
        MiddlewareOp::Prepare { file, packet_only } => match read_middleware_request(file) {
            Ok(request) => match innen_core::middleware::prepare(request) {
                Ok(prepared) => {
                    if *packet_only {
                        println!("{}", prepared.packet_json);
                    } else if is_human(format) {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&prepared)
                                .expect("middleware output serializes")
                        );
                    } else {
                        println!(
                            "{}",
                            serde_json::to_string(&prepared).expect("middleware output serializes")
                        );
                    }
                    0
                }
                Err(error) => print_middleware_unknown(error, *packet_only),
            },
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        MiddlewareOp::Expand {
            file,
            expect_source_sha256,
            ids,
        } => match read_middleware_request(file) {
            Ok(request) => match innen_core::middleware::expand(request, expect_source_sha256, ids)
            {
                Ok(output) => {
                    println!(
                        "{}",
                        if is_human(format) {
                            serde_json::to_string_pretty(&output)
                                .expect("middleware expansion serializes")
                        } else {
                            serde_json::to_string(&output).expect("middleware expansion serializes")
                        }
                    );
                    0
                }
                Err(error) => print_middleware_unknown(error, false),
            },
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
    }
}

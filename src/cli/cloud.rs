#[derive(clap::Subcommand)]
pub(super) enum CloudOp {
    /// `rclone lsd <remote>:` (canned `canned-dir` under the test stub).
    Status(CloudStatusArgs),
    /// `rclone version` (stub reports canned version).
    Doctor,
}
#[derive(clap::Args)]
pub(super) struct CloudStatusArgs {
    /// Remote name (stub ignores the value).
    #[arg(long, default_value = "myremote")]
    pub(super) remote: String,
}

#[derive(serde::Serialize)]
pub(super) struct CloudStatusJson {
    pub(super) remote: String,
    pub(super) output: String,
}

#[derive(serde::Serialize)]
pub(super) struct CloudDoctorJson {
    pub(super) output: String,
}

use super::util::is_human;
use std::path::PathBuf;

// P2 `cloud status [--remote <name>]` (default `myremote` so bare
// `cloud status` works against the stub).

// --- P2 JSON wire structs (field order is the wire order) ---

pub(super) fn cmd_cloud_status(format: &str, remote: &str) -> i32 {
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

/// Resolve the `rclone` binary via PATH lookup (CLI-only; unit tests
/// inject the fixture path directly into `Rclone { bin }`).
pub(super) fn resolve_rclone_bin() -> Result<PathBuf, String> {
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

pub(super) fn cmd_cloud_doctor(format: &str) -> i32 {
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

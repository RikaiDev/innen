use super::*;
pub(super) enum Activity {
    Idle,
    Active,
    Unknown(String),
}

#[cfg(target_os = "linux")]
pub(super) fn session_activity(targets: &[PathBuf]) -> Activity {
    let canonical = targets
        .iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect::<Vec<_>>();
    let processes = match fs::read_dir("/proc") {
        Ok(processes) => processes,
        Err(error) => return Activity::Unknown(format!("cannot inspect /proc: {error}")),
    };
    for process in processes.flatten() {
        if !process
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let fds = match fs::read_dir(process.path().join("fd")) {
            Ok(fds) => fds,
            Err(_) => continue,
        };
        for fd in fds.flatten() {
            let Ok(open_path) = fs::read_link(fd.path()) else {
                continue;
            };
            if canonical
                .iter()
                .any(|target| open_path == *target || open_path.starts_with(target))
            {
                return Activity::Active;
            }
        }
    }
    Activity::Idle
}

#[cfg(target_os = "macos")]
pub(super) fn session_activity(targets: &[PathBuf]) -> Activity {
    let mut command = Command::new("/usr/sbin/lsof");
    command.arg("-Fn");
    for target in targets {
        command.arg(target);
    }
    match command.output() {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => Activity::Active,
        Ok(output) if output.status.code() == Some(1) && output.stdout.is_empty() => Activity::Idle,
        Ok(output) => Activity::Unknown(format!("lsof exited {}", output.status)),
        Err(error) => Activity::Unknown(format!("lsof failed: {error}")),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn session_activity(_targets: &[PathBuf]) -> Activity {
    Activity::Unknown("open-handle inspection is unsupported on this platform".into())
}

#[cfg(unix)]
pub(super) fn snapshot_target_identities(
    targets: &[PathBuf],
) -> Result<Vec<TargetIdentity>, Error> {
    targets
        .iter()
        .map(|path| {
            let metadata = fs::symlink_metadata(path).map_err(|error| io(path, error))?;
            if metadata.file_type().is_symlink() {
                return Err(Error::Purge(format!(
                    "purge target is a symlink: {}",
                    path.display()
                )));
            }
            Ok(TargetIdentity {
                path: path.clone(),
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        })
        .collect()
}

#[cfg(not(unix))]
pub(super) fn snapshot_target_identities(
    _targets: &[PathBuf],
) -> Result<Vec<TargetIdentity>, Error> {
    Err(Error::Purge(
        "filesystem identity verification is unsupported on this platform".into(),
    ))
}

#[cfg(unix)]
pub(super) fn verify_target_identities(expected: &[TargetIdentity]) -> Result<(), Error> {
    for identity in expected {
        let metadata =
            fs::symlink_metadata(&identity.path).map_err(|error| io(&identity.path, error))?;
        if metadata.file_type().is_symlink()
            || metadata.dev() != identity.device
            || metadata.ino() != identity.inode
        {
            return Err(Error::Purge(format!(
                "purge target identity changed: {}",
                identity.path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn verify_target_identities(_expected: &[TargetIdentity]) -> Result<(), Error> {
    Err(Error::Purge(
        "filesystem identity verification is unsupported on this platform".into(),
    ))
}

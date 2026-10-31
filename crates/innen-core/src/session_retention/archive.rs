use super::*;
pub(super) fn candidate_targets(candidate: &Candidate) -> Vec<PathBuf> {
    if candidate.source == Source::Antigravity {
        antigravity_targets(&candidate.path, &candidate.id)
    } else {
        vec![candidate.path.clone()]
    }
}

pub(super) fn find_candidate(
    source: Source,
    session_id: &str,
    source_root: Option<&Path>,
) -> Result<Candidate, Error> {
    let id = sources::validate_id(session_id).map_err(|error| Error::Proof(error.to_string()))?;
    find_all_candidates(Some(source), source_root)
        .map_err(|error| Error::Conversation(error.to_string()))?
        .into_iter()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| Error::Proof(format!("session not found: {}:{id}", source.as_str())))
}

pub(super) struct Bundle {
    pub(super) sha256: String,
    pub(super) bytes: u64,
    pub(super) targets: Vec<PathBuf>,
}

pub(super) fn bundle(candidate: &Candidate) -> Result<Bundle, Error> {
    if candidate.source == Source::Opencode {
        let rows = sources::load_database_session(&candidate.path, &candidate.id)
            .map_err(|error| Error::Conversation(error.to_string()))?;
        let bytes = serde_json::to_vec(&rows).expect("database export serializes");
        return Ok(Bundle {
            sha256: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
            targets: vec![candidate.path.clone()],
        });
    }
    let targets = if candidate.source == Source::Antigravity {
        antigravity_targets(&candidate.path, &candidate.id)
    } else {
        let (sha256, bytes) = hash_file(&candidate.path)?;
        return Ok(Bundle {
            sha256,
            bytes,
            targets: vec![candidate.path.clone()],
        });
    };
    hash_targets(&targets)
}

pub(super) fn antigravity_targets(path: &Path, id: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut brain_session = path
        .ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some(id))
        .map(Path::to_path_buf);
    if brain_session.is_none()
        && path.file_stem().and_then(|name| name.to_str()) == Some(id)
        && path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "conversations")
    {
        brain_session = path
            .parent()
            .and_then(Path::parent)
            .map(|store| store.join("brain").join(id));
        if path.exists() {
            out.push(path.to_owned());
        }
    }
    if let Some(brain_session) = brain_session {
        if brain_session.exists() {
            out.push(brain_session.clone());
        }
        if let Some(brain_root) = brain_session.parent() {
            if brain_root.file_name().is_some_and(|name| name == "brain") {
                if let Some(store_root) = brain_root.parent() {
                    let conversation = store_root.join("conversations").join(format!("{id}.db"));
                    if conversation.exists() {
                        out.push(conversation);
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

pub(super) fn hash_targets(targets: &[PathBuf]) -> Result<Bundle, Error> {
    if targets.is_empty() {
        return Err(Error::Proof(
            "session bundle has no existing targets".into(),
        ));
    }
    let mut files = Vec::new();
    for target in targets {
        collect(target, target, &mut files)?;
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    for (name, path) in files {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        let mut file = File::open(&path).map_err(|error| io(&path, error))?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|error| io(&path, error))?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            hasher.update(&buffer[..count]);
        }
    }
    let digest = hasher.finalize();
    Ok(Bundle {
        sha256: hex_digest(&digest),
        bytes,
        targets: targets.to_vec(),
    })
}

fn collect(base: &Path, path: &Path, files: &mut Vec<(String, PathBuf)>) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io(path, error))?;
    if metadata.file_type().is_symlink() {
        // A symlink that points inside the bundle (a `node_modules/.bin` entry,
        // for example) is part of the session's own bytes and is removed with
        // it. One that points outside could delete data the session never
        // owned, so it stays refused.
        if !points_inside(base, path) {
            return Err(Error::Proof(format!(
                "symlink inside purge bundle is unsupported: {}",
                path.display()
            )));
        }
    }
    if metadata.is_file() {
        let name = path
            .strip_prefix(base.parent().unwrap_or(base))
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        files.push((name, path.to_owned()));
    } else if metadata.is_dir() {
        let mut entries = fs::read_dir(path)
            .map_err(|error| io(path, error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io(path, error))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            collect(base, &entry.path(), files)?;
        }
    }
    Ok(())
}

/// Whether a symlink's target resolves inside the bundle it belongs to.
fn points_inside(base: &Path, link: &Path) -> bool {
    let Ok(target) = fs::read_link(link) else {
        return false;
    };
    let resolved = if target.is_absolute() {
        target
    } else {
        match link.parent() {
            Some(parent) => parent.join(target),
            None => return false,
        }
    };
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let resolved = resolved.canonicalize().unwrap_or(resolved);
    resolved.starts_with(&base)
}

fn hash_file(path: &Path) -> Result<(String, u64), Error> {
    let file = File::open(path).map_err(|error| io(path, error))?;
    hash_reader(file, path)
}

fn hash_reader(mut reader: impl Read, path: &Path) -> Result<(String, u64), Error> {
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(|error| io(path, error))?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    Ok((hex_digest(&digest), bytes))
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A `node_modules/.bin` entry points inside the session's own scratch
    /// directory; it is part of the session's bytes and goes with it.
    #[test]
    fn a_symlink_that_stays_inside_the_bundle_is_part_of_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("session");
        let scratch = session.join("scratch").join("node_modules");
        fs::create_dir_all(scratch.join(".bin")).unwrap();
        fs::write(scratch.join("core.js"), b"module.exports = 1;").unwrap();
        symlink(
            scratch.join("core.js"),
            scratch.join(".bin").join("core"),
        )
        .unwrap();

        let bundle = hash_targets(&[session.clone()]).expect("the bundle hashes");
        assert!(bundle.bytes > 0);
    }

    /// A symlink that points outside the bundle could delete data the session
    /// never owned, so it stays refused.
    #[test]
    fn a_symlink_that_leaves_the_bundle_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.txt");
        fs::write(&outside, b"not the session's bytes").unwrap();
        let session = dir.path().join("session");
        fs::create_dir_all(&session).unwrap();
        symlink(&outside, session.join("escape")).unwrap();

        let error = match hash_targets(&[session.clone()]) {
            Ok(_) => panic!("an escaping symlink must be refused"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("symlink inside purge bundle"));
    }
}

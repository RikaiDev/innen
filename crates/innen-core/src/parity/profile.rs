//! Profile page render.

use std::path::Path;

use crate::toml::string_entries;

/// Keys this page renders. Every other key in `profile.toml` is ignored.
const PROFILE_KEYS: [&str; 3] = ["title", "blurb", "status"];

/// Read the recognised keys from flat `profile/profile.toml`.
///
/// Values that are not double-quoted strings are skipped without error, and a
/// missing key reads as an empty string. Section headers are not expected in
/// this file and are ignored by the shared reader.
pub(crate) fn profile_string_map(text: &str) -> std::collections::HashMap<String, String> {
    string_entries(text)
        .into_iter()
        .filter(|(key, _)| PROFILE_KEYS.contains(&key.as_str()))
        .collect()
}

/// Render the profile page.
///
/// Byte-exact `"# {title}\n\n{blurb}\n\nstatus: {status}\n"`; absent keys are
/// empty strings. A missing or unreadable file errors with prefix
/// `"missing profile/profile.toml"`.
pub fn profile_render(root: &Path) -> Result<String, String> {
    let path = root.join("profile/profile.toml");
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("missing profile/profile.toml: {e}"))?;
    let map = profile_string_map(&text);
    let title = map.get("title").map(String::as_str).unwrap_or("");
    let blurb = map.get("blurb").map(String::as_str).unwrap_or("");
    let status = map.get("status").map(String::as_str).unwrap_or("");
    Ok(format!("# {title}\n\n{blurb}\n\nstatus: {status}\n"))
}

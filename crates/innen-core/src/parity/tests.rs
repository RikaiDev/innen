use super::profile::profile_string_map;
use super::*;
use crate::journal::Journal;

fn project_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "t:1", "type": "Task", "label": "task one"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "d:1", "type": "Decision", "label": "decision one"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "e:1", "type": "Experiment", "label": "experiment one"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "ds:1", "type": "Dataset", "label": "dataset one"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "u:1", "type": "Task", "label": "unrelated chores"}),
    )
    .unwrap();
    for from in ["t:1", "d:1", "e:1", "ds:1"] {
        j.append(
            "edge.assert",
            &serde_json::json!({"from": from, "type": "BELONGS_TO", "to": "p:x"}),
        )
        .unwrap();
    }
    dir
}

#[test]
fn guide_mentions_query_first() {
    let text = guide_text();
    assert!(text.contains("query"));
    assert!(text.find("query") < text.find("status"));
}
#[test]
fn timeline_filters_by_month() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "a:1", "type": "Task", "label": "aug"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "s:1", "type": "Task", "label": "sep"}),
    )
    .unwrap();
    // No Journal API supports backdating: `append` stamps wall-clock
    // observed_utc and ignores any payload observed_utc, so both rows
    // would share today's month. Rewrite the stored envelope timestamps
    // to give the filter two distinct months; assertions below are
    // unchanged in intent.
    let jpath = dir.path().join(".innen/journal.jsonl");
    let content = std::fs::read_to_string(&jpath).unwrap();
    let mut lines: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    lines[0]["observed_utc"] = serde_json::json!("2026-08-01T00:00:00Z");
    lines[1]["observed_utc"] = serde_json::json!("2026-09-01T00:00:00Z");
    let out = lines
        .iter()
        .map(|v| serde_json::to_string(v).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&jpath, out).unwrap();
    let rows = timeline(dir.path(), Some("2026-09"));
    assert_eq!(rows.len(), 1);
    assert!(rows[0].observed_utc.starts_with("2026-09"));
}
#[test]
fn project_render_groups_by_kind() {
    let dir = project_fixture();
    let out = project_render(dir.path(), "p:x").unwrap();
    for section in ["## Decisions", "## Tasks", "## Experiments", "## Datasets"] {
        assert!(out.contains(section), "missing {section}");
    }
    assert!(!out.contains("unrelated"));
}
#[test]
fn project_unknown_id_errors() {
    let dir = project_fixture();
    let err = project_render(dir.path(), "p:nope").unwrap_err();
    assert_eq!(err, "unknown project: p:nope");
}
#[test]
fn project_render_resolves_shorthand_id() {
    let dir = project_fixture();
    // Fixture defines "p:x"; passing shorthand "x" should resolve to "p:x".
    let out = project_render(dir.path(), "x").unwrap();
    assert!(out.contains("# Project X"));
    assert!(out.contains("## Tasks"));
}
#[test]
fn profile_renders_fixture_toml() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("profile")).unwrap();
    std::fs::write(
        dir.path().join("profile/profile.toml"),
        "title = \"T\"\nblurb = \"B\"\nstatus = \"active\"\nextra = \"ignored\"\n",
    )
    .unwrap();
    let out = profile_render(dir.path()).unwrap();
    assert_eq!(out, "# T\n\nB\n\nstatus: active\n");
}
#[test]
fn profile_missing_file_errors() {
    let dir = tempfile::tempdir().unwrap();
    let err = profile_render(dir.path()).unwrap_err();
    assert!(
        err.starts_with("missing profile/profile.toml"),
        "got: {err}"
    );
}
#[test]
fn profile_escapes_quote_and_backslash() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("profile")).unwrap();
    // TOML bytes: title = "a\"b\\c" -> value a"b\c
    std::fs::write(
        dir.path().join("profile/profile.toml"),
        "title = \"a\\\"b\\\\c\"\nblurb = \"B\"\nstatus = \"s\"\n",
    )
    .unwrap();
    let text = std::fs::read_to_string(dir.path().join("profile/profile.toml")).unwrap();
    let map = profile_string_map(&text);
    assert_eq!(map.get("title").map(String::as_str), Some("a\"b\\c"));
    let out = profile_render(dir.path()).unwrap();
    assert_eq!(out, "# a\"b\\c\n\nB\n\nstatus: s\n");
}
#[test]
fn profile_hash_inside_quotes_kept_outside_stripped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("profile")).unwrap();
    std::fs::write(
            dir.path().join("profile/profile.toml"),
            "# leading comment\ntitle = \"a#b\" # trailing comment\nblurb = \"B\"#no-space\nstatus = \"ok\"\n",
        )
        .unwrap();
    let out = profile_render(dir.path()).unwrap();
    assert_eq!(out, "# a#b\n\nB\n\nstatus: ok\n");
}
#[test]
fn profile_other_section_header_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("profile")).unwrap();
    std::fs::write(
        dir.path().join("profile/profile.toml"),
        "title = \"T\"\n[other]\nblurb = \"B\"\nstatus = \"active\"\nfoo = \"bar\"\n",
    )
    .unwrap();
    let out = profile_render(dir.path()).unwrap();
    assert_eq!(out, "# T\n\nB\n\nstatus: active\n");
}
#[test]
fn profile_duplicate_keys_last_wins_bare_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("profile")).unwrap();
    std::fs::write(
        dir.path().join("profile/profile.toml"),
        "title = \"first\"\ntitle = \"second\"\nblurb = \"B\"\nstatus = \"s\"\ncount = 3\n",
    )
    .unwrap();
    let out = profile_render(dir.path()).unwrap();
    assert_eq!(out, "# second\n\nB\n\nstatus: s\n");
}
#[test]
fn profile_missing_keys_render_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("profile")).unwrap();
    std::fs::write(dir.path().join("profile/profile.toml"), "title = \"T\"\n").unwrap();
    let out = profile_render(dir.path()).unwrap();
    assert_eq!(out, "# T\n\n\n\nstatus: \n");
}
#[test]
fn project_retracted_belongs_to_skipped() {
    let dir = project_fixture();
    let j = Journal::open(dir.path()).unwrap();
    j.append(
        "edge.retract",
        &serde_json::json!({"from": "t:1", "type": "BELONGS_TO", "to": "p:x"}),
    )
    .unwrap();
    let out = project_render(dir.path(), "p:x").unwrap();
    assert!(!out.contains("task one"), "retracted member skipped: {out}");
    assert!(out.contains("decision one"));
}
#[test]
fn project_label_missing_falls_back_to_id() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "t:nolabel", "type": "Task"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "t:empty", "type": "Task", "label": ""}),
    )
    .unwrap();
    for from in ["t:nolabel", "t:empty"] {
        j.append(
            "edge.assert",
            &serde_json::json!({"from": from, "type": "BELONGS_TO", "to": "p:x"}),
        )
        .unwrap();
    }
    let out = project_render(dir.path(), "p:x").unwrap();
    assert!(
        out.contains("- t:nolabel"),
        "missing label falls back to id: {out}"
    );
    assert!(
        out.contains("- t:empty"),
        "empty label falls back to id: {out}"
    );
}
#[test]
fn project_members_sorted_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "t:z", "type": "Task", "label": "zeta"}),
    )
    .unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "t:a", "type": "Task", "label": "alpha"}),
    )
    .unwrap();
    for from in ["t:z", "t:a"] {
        j.append(
            "edge.assert",
            &serde_json::json!({"from": from, "type": "BELONGS_TO", "to": "p:x"}),
        )
        .unwrap();
    }
    let out = project_render(dir.path(), "p:x").unwrap();
    let pos_alpha = out.find("alpha").expect("alpha present");
    let pos_zeta = out.find("zeta").expect("zeta present");
    assert!(pos_alpha < pos_zeta, "members sorted: {out}");
}
#[test]
fn project_custom_task_counted_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
    )
    .unwrap();
    for (id, ty, label) in [
        ("t:lower", "task", "lower task"),
        ("t:upper", "TASK", "upper task"),
        ("t:mixed", "TaSk", "mixed task"),
    ] {
        j.append(
            "node.upsert",
            &serde_json::json!({"id": id, "type": ty, "label": label}),
        )
        .unwrap();
        j.append(
            "edge.assert",
            &serde_json::json!({"from": id, "type": "BELONGS_TO", "to": "p:x"}),
        )
        .unwrap();
    }
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "n:other", "type": "Note", "label": "ignored note"}),
    )
    .unwrap();
    j.append(
        "edge.assert",
        &serde_json::json!({"from": "n:other", "type": "BELONGS_TO", "to": "p:x"}),
    )
    .unwrap();
    let out = project_render(dir.path(), "p:x").unwrap();
    assert!(
        out.contains("- lower task"),
        "lowercase task counted: {out}"
    );
    assert!(
        out.contains("- upper task"),
        "uppercase task counted: {out}"
    );
    assert!(
        out.contains("- mixed task"),
        "mixed-case task counted: {out}"
    );
    assert!(
        !out.contains("ignored note"),
        "unknown kinds still ignored: {out}"
    );
}
#[test]
fn project_empty_sections_still_emitted() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    j.append(
        "node.upsert",
        &serde_json::json!({"id": "p:x", "type": "Project", "label": "Project X"}),
    )
    .unwrap();
    let out = project_render(dir.path(), "p:x").unwrap();
    for section in ["## Decisions", "## Tasks", "## Experiments", "## Datasets"] {
        assert!(out.contains(section), "missing {section}: {out}");
    }
    assert_eq!(
        out,
        "# Project X\n\n## Decisions\n\n## Tasks\n\n## Experiments\n\n## Datasets\n"
    );
}

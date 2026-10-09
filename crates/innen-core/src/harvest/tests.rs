
use super::*;
use crate::tap::watermark_path;

fn write_inbox(dir: &tempfile::TempDir, files: &[(&str, &str)]) {
    let inbox = dir.path().join("00-inbox/harvest");
    std::fs::create_dir_all(&inbox).unwrap();
    for (name, content) in files {
        std::fs::write(inbox.join(name), content).unwrap();
    }
}

#[test]
fn harvest_picks_new_files_only() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(&dir, &[("a.md", "hello"), ("b.md", "world")]);
    let r1 = check(dir.path());
    assert_eq!(
        r1.taps[0].new_files,
        vec!["a.md".to_string(), "b.md".to_string()]
    );
    let ing = ingest::run(dir.path());
    assert_eq!(ing.added, 2);
    assert!(ing.skipped.is_empty());
    let r2 = check(dir.path());
    assert!(r2.taps.iter().all(|t| t.new_files.is_empty()));
}

#[test]
fn credential_file_skipped_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(
        &dir,
        &[
            ("ok.md", "hello"),
            ("bad.md", "key AKIAIOSFODNN7EXAMPLE end"),
        ],
    );
    let ing = ingest::run(dir.path());
    assert_eq!(ing.added, 1);
    assert_eq!(ing.skipped.len(), 1);
    assert_eq!(ing.skipped[0].path, "bad.md");
    assert_eq!(ing.skipped[0].pattern, "aws-access-key");
    assert_eq!(ing.skipped[0].preview, "AKIAIO***");
    let journal = std::fs::read_to_string(dir.path().join(".innen/journal.jsonl")).unwrap();
    assert!(!journal.contains("bad.md"));
}

#[test]
fn ingest_skips_openai_and_upstash_with_labels() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(
        &dir,
        &[
            ("ok.md", "hello"),
            (
                "ai.md",
                "token sk-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnop here",
            ),
            ("up.md", "UPSTASH_REDIS_REST_TOKEN=hunter2valuepayload end"),
        ],
    );
    let ing = ingest::run(dir.path());
    assert_eq!(ing.added, 1);
    assert_eq!(ing.skipped.len(), 2);
    assert_eq!(ing.skipped[0].pattern, "openai-key");
    assert_eq!(ing.skipped[0].preview, "sk-ABC***");
    assert_eq!(ing.skipped[1].pattern, "upstash-token");
    assert!(ing.skipped[1].preview.starts_with("UPSTAS"));
    let journal = std::fs::read_to_string(dir.path().join(".innen/journal.jsonl")).unwrap();
    assert!(!journal.contains("ai.md"));
    assert!(!journal.contains("up.md"));
}

#[test]
fn ingest_records_the_consumed_file_names() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(&dir, &[("a.md", "x")]);
    ingest::run(dir.path());
    // The cursor names its inputs rather than counting them, which is what
    // keeps a shrinking inbox from swallowing new files.
    let state = crate::tap::load_cursor(dir.path(), "harvest-dir");
    assert_eq!(
        state,
        crate::tap::CursorState::Current(crate::tap::Cursor {
            consumed: ["a.md".to_string()].into_iter().collect()
        })
    );
    let ing2 = ingest::run(dir.path());
    assert_eq!(ing2.added, 0);
}

#[test]
fn skipped_files_are_consumed_and_do_not_stall_the_queue() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(
        &dir,
        &[
            ("bad.md", "key AKIAIOSFODNN7EXAMPLE end"),
            ("ok.md", "hello"),
        ],
    );
    let ing = ingest::run(dir.path());
    assert_eq!(ing.added, 1);
    assert_eq!(ing.skipped.len(), 1);
    let consumed = crate::tap::load_cursor(dir.path(), "harvest-dir")
        .known()
        .expect("cursor names its inputs")
        .clone();
    assert!(
        consumed.contains("bad.md"),
        "skipped files are consumed too"
    );
    assert!(consumed.contains("ok.md"));
}

#[test]
fn check_empty_inbox_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("00-inbox/harvest")).unwrap();
    let r = check(dir.path());
    assert!(r.taps.iter().all(|t| t.new_files.is_empty()));
    assert!(r.taps.iter().all(|t| t.skipped.is_empty()));
}

#[test]
fn check_ingest_agree_on_clean_files() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(&dir, &[("a.md", "hello"), ("b.md", "world")]);
    let r = check(dir.path());
    assert_eq!(
        r.taps[0].new_files,
        vec!["a.md".to_string(), "b.md".to_string()]
    );
    let ing = ingest::run(dir.path());
    assert_eq!(ing.added, 2);
    assert!(ing.skipped.is_empty());
}

#[test]
fn deleting_ingested_files_never_hides_new_ones() {
    // The failure this replaces: a count cursor plus post-ingest deletion.
    // Each deletion shifted the window by one, and once the count passed
    // the listing size `harvest --check` reported an empty backlog while
    // unprocessed files sat in the inbox — silently, for 17 days.
    let dir = tempfile::tempdir().unwrap();
    write_inbox(&dir, &[("a.md", "one"), ("b.md", "two")]);
    assert_eq!(ingest::run(dir.path()).added, 2);
    // The pending files' own instructions say to delete them after ingest.
    std::fs::remove_file(dir.path().join("00-inbox/harvest/a.md")).unwrap();
    std::fs::remove_file(dir.path().join("00-inbox/harvest/b.md")).unwrap();

    write_inbox(&dir, &[("c.md", "three"), ("d.md", "four")]);
    let r = check(dir.path());
    assert_eq!(
        r.taps[0].new_files,
        vec!["c.md".to_string(), "d.md".to_string()],
        "a shrunk inbox must not shift the window past new files"
    );
    assert_eq!(r.taps[0].cursor, "current");
    assert_eq!(ingest::run(dir.path()).added, 2);
    assert!(check(dir.path()).taps[0].new_files.is_empty());
}

#[test]
fn many_ingest_delete_cycles_keep_finding_new_files() {
    let dir = tempfile::tempdir().unwrap();
    for round in 0..12 {
        let name = format!("pending-{round:04}.md");
        write_inbox(&dir, &[(name.as_str(), &format!("body {round}"))]);
        assert_eq!(
            ingest::run(dir.path()).added,
            1,
            "round {round} must ingest its new file"
        );
        std::fs::remove_file(dir.path().join("00-inbox/harvest").join(&name)).unwrap();
    }
    assert!(check(dir.path()).taps[0].new_files.is_empty());
}

#[test]
fn a_legacy_count_cursor_rebuilds_and_says_so() {
    // The on-disk state this machine was stuck in: a count of 875 with 96
    // files present. It must re-list the inbox and report the rebuild
    // rather than presenting an empty backlog.
    let dir = tempfile::tempdir().unwrap();
    write_inbox(&dir, &[("a.md", "one"), ("b.md", "two")]);
    let path = watermark_path(dir.path(), "harvest-dir");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"875").unwrap();

    let r = check(dir.path());
    assert_eq!(r.taps[0].cursor, "legacy_count_rebuilt");
    assert_eq!(
        r.taps[0].new_files,
        vec!["a.md".to_string(), "b.md".to_string()]
    );

    let ing = ingest::run(dir.path());
    assert_eq!(ing.added, 2);
    assert_eq!(check(dir.path()).taps[0].cursor, "current");
    assert!(check(dir.path()).taps[0].new_files.is_empty());
}

#[test]
fn consumed_names_survive_a_file_returning_to_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    write_inbox(&dir, &[("a.md", "same bytes")]);
    assert_eq!(ingest::run(dir.path()).added, 1);
    // Same name, same content, back in the inbox: the cursor still knows
    // it, so a retry loop cannot append it twice.
    write_inbox(&dir, &[("a.md", "same bytes")]);
    assert_eq!(ingest::run(dir.path()).added, 0);
    let journal = std::fs::read_to_string(dir.path().join(".innen/journal.jsonl")).unwrap();
    assert_eq!(journal.lines().count(), 1);
}

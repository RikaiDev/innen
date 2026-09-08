use super::lexical::{bm25, lex, tokens};
use super::source::file_passages;
#[test]
fn bm25_preserves_relevance_instead_of_alphabetical_seed_order() {
    let docs = vec![
        lex("a".into(), "deploy unrelated"),
        lex("z".into(), "release deploy release"),
    ];
    assert_eq!(bm25(&docs, &tokens("release deploy"))[0].0, 1);
}

#[test]
fn mixed_code_and_cjk_clues_share_the_same_tokens() {
    let terms = tokens("Release_main 部署授權");
    assert!(terms.contains(&"release".into()));
    assert!(terms.contains(&"main".into()));
    assert!(terms.contains(&"部署".into()));
    assert!(terms.contains(&"授權".into()));
    assert!(tokens("").is_empty());
    assert!(tokens("!!!").is_empty());
}
#[test]
fn file_offsets_are_exact_across_unicode_and_long_lines() {
    let text = format!("標題\n{}\n末尾", "字".repeat(3000));
    for p in file_passages(&text, 100).passages {
        assert_eq!(
            &text[p.start_byte.unwrap() as usize..p.end_byte.unwrap() as usize],
            p.text
        );
    }
}

#[test]
fn file_passage_allocation_stops_at_the_record_budget() {
    let scan = file_passages(&"x\n".repeat(100_000), 2);
    assert_eq!(scan.records, 2);
    assert_eq!(scan.passages.len(), 2);
    assert!(scan.truncated);
}

#[test]
fn short_identity_catalog_is_isolated_by_source_root() {
    use super::source::resolve_locator;
    use super::types::{Locator, Options};
    use std::collections::BTreeMap;
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let ids = [
        "123e4567-e89b-12d3-a456-426614174000",
        "123e4567-e89b-12d3-a456-426614174001",
    ];
    for (root, count) in [(a.path(), 1), (b.path(), 2)] {
        for id in &ids[..count] {
            std::fs::write(root.join(format!("{id}.jsonl")),serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":"/fixture","timestamp":"2026-01-01T00:00:00Z"}}).to_string()+"\n").unwrap();
        }
    }
    let loc = |root: &std::path::Path| Locator::Conversation {
        source: "codex".into(),
        session_id: "123e4567".into(),
        source_root: Some(root.display().to_string()),
    };
    let mut catalog = BTreeMap::new();
    let mut count = 0;
    assert!(resolve_locator(
        &loc(a.path()),
        &Options::default(),
        &mut catalog,
        &mut count
    )
    .is_ok());
    assert!(resolve_locator(
        &loc(b.path()),
        &Options::default(),
        &mut catalog,
        &mut count
    )
    .is_err());
}

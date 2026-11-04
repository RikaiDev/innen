use assert_cmd::Command;
use innen_core::journal::Journal;
use serde_json::{json, Value};

const SHELL_HOSTILE: &str = "資委會是針對台大報告！跟全台據點有什麼關係？ (a*b) #1 $HOME `x`";

fn root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    Journal::open(dir.path())
        .unwrap()
        .append(
            "node.upsert",
            &json!({"id":"project:ntuh","type":"Project","label":"台大雲林專案"}),
        )
        .unwrap();
    dir
}

fn innen(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("innen").unwrap();
    c.arg("--root").arg(dir);
    c
}

#[test]
fn stdin_text_survives_shell_characters_and_surfaces_in_query() {
    let dir = root();
    let out = innen(dir.path())
        .args(["constraint", "add", "--project", "project:ntuh"])
        .args(["--source", "agy:s1#user-turn-30", "--text-file", "-"])
        .write_stdin(SHELL_HOSTILE)
        .assert()
        .success();
    let created: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(created["id"]
        .as_str()
        .unwrap()
        .starts_with("constraint:ntuh:"));

    let q = innen(dir.path())
        .args(["query", "台大雲林專案", "--format", "human"])
        .assert()
        .success();
    let text = String::from_utf8(q.get_output().stdout.clone()).unwrap();
    assert!(text.contains("Binding constraints (1 of 1"), "{text}");
    assert!(
        text.contains(SHELL_HOSTILE) && text.contains("agy:s1#user-turn-30"),
        "{text}"
    );
}

#[test]
fn recording_the_same_words_twice_keeps_one_constraint() {
    let dir = root();
    for _ in 0..2 {
        innen(dir.path())
            .args(["constraint", "add", "--project", "project:ntuh"])
            .args(["--source", "s", "--text", "只談台大"])
            .assert()
            .success();
    }
    let q = innen(dir.path())
        .args(["query", "台大雲林專案", "--format", "human"])
        .assert()
        .success();
    let text = String::from_utf8(q.get_output().stdout.clone()).unwrap();
    assert!(text.contains("Binding constraints (1 of 1"), "{text}");
}

#[test]
fn missing_source_or_project_is_refused() {
    let dir = root();
    innen(dir.path())
        .args([
            "constraint",
            "add",
            "--project",
            "project:ntuh",
            "--source",
            " ",
            "--text",
            "x",
        ])
        .assert()
        .failure();
    innen(dir.path())
        .args([
            "constraint",
            "add",
            "--project",
            "project:nope",
            "--source",
            "s",
            "--text",
            "x",
        ])
        .assert()
        .failure();
}

use assert_cmd::Command;
use serde_json::json;

#[test]
fn finalize_is_dry_run_then_publishes_final_only() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("03-output/quotations");
    std::fs::create_dir_all(&output).unwrap();
    let final_pdf = output.join("final.pdf");
    let draft_pdf = output.join("draft.pdf");
    std::fs::write(&final_pdf, b"final bytes").unwrap();
    std::fs::write(&draft_pdf, b"draft bytes").unwrap();
    let plan = root.path().join("plan.json");
    std::fs::write(&plan, json!({
        "format": "innen.final-collection.v1",
        "collection": "quotes-2026",
        "final_files": [{"path":final_pdf,"sha256":innen_core::ids::sha256_hex(b"final bytes")}],
        "remove_files": [{"path":draft_pdf,"sha256":innen_core::ids::sha256_hex(b"draft bytes")}],
    }).to_string()).unwrap();
    let root_arg = root.path().to_str().unwrap();
    let plan_arg = plan.to_str().unwrap();
    let dry = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "--root", root_arg, "artifact", "finalize", "--plan", plan_arg,
        ])
        .assert()
        .code(0);
    let dry_json: serde_json::Value = serde_json::from_slice(&dry.get_output().stdout).unwrap();
    assert_eq!(dry_json["applied"], false);
    assert!(final_pdf.exists() && draft_pdf.exists());
    let applied = Command::cargo_bin("innen")
        .unwrap()
        .args([
            "--root", root_arg, "artifact", "finalize", "--plan", plan_arg, "--apply",
        ])
        .assert()
        .code(0);
    let applied_json: serde_json::Value =
        serde_json::from_slice(&applied.get_output().stdout).unwrap();
    assert_eq!(applied_json["applied"], true);
    assert!(!final_pdf.exists() && !draft_pdf.exists());
    assert_eq!(
        std::fs::read(
            root.path()
                .join("03-output/artifacts/collections/quotes-2026/final.pdf")
        )
        .unwrap(),
        b"final bytes"
    );
}

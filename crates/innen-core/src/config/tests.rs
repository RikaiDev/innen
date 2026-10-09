//! Config precedence and TOML-limit tests.

use std::collections::HashMap;

use super::*;

use std::fs;

#[test]
fn config_precedence_env_over_toml() {
    // innen.toml sets format=json, env INNEN_FORMAT=human → human wins.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("innen.toml"), "[core]\nformat = \"json\"\n")
        .expect("write innen.toml");
    std::env::set_var("INNEN_FORMAT", "human");
    let (cfg, _warnings) = load(dir.path(), &CliOverrides::default());
    std::env::remove_var("INNEN_FORMAT");
    assert_eq!(cfg.format, "human", "env must beat toml: {cfg:?}");
}

#[test]
fn config_machine_warns_on_override() {
    // machine.json sets format + toml sets format → warning string emitted.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("innen.toml"), "[core]\nformat = \"json\"\n")
        .expect("write innen.toml");
    let innen_dir = dir.path().join(".innen");
    fs::create_dir_all(&innen_dir).expect("mkdir .innen");
    fs::write(innen_dir.join("machine.json"), r#"{"format": "human"}"#)
        .expect("write machine.json");
    let env: HashMap<String, String> = HashMap::new();
    let (cfg, warnings) = load_with_env(dir.path(), &CliOverrides::default(), &env);
    assert_eq!(cfg.format, "human", "machine.json must beat toml: {cfg:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w == "warning: machine.json overrides innen.toml: format"),
        "must emit exact override warning, got: {warnings:?}"
    );
}

#[test]
fn resolve_root_precedence_and_error_cases() {
    let home = tempfile::tempdir().expect("tempdir");
    let kb_dir = tempfile::tempdir().expect("tempdir");
    let kb_path = kb_dir.path().to_path_buf();
    let mut env = HashMap::new();
    env.insert(
        "XDG_CONFIG_HOME".to_string(),
        home.path().to_string_lossy().into_owned(),
    );

    // 1. Unconfigured fails without mutation
    let err = resolve_root_with_env(None, Some(&env)).unwrap_err();
    assert!(matches!(err, RootResolutionError::Unconfigured));

    // 2. Global bootstrap registration requires absolute path
    let rel_err = set_global_value_with_env("root", "relative/path", Some(&env)).unwrap_err();
    assert!(matches!(rel_err, ConfigSetError::NotAbsolute(_)));

    // 3. Global bootstrap sets root and preserves unrelated keys
    set_global_value_with_env("format", "json", Some(&env)).expect("set format");
    set_global_value_with_env("root", &kb_path.to_string_lossy(), Some(&env)).expect("set root");
    let got_root = resolve_root_with_env(None, Some(&env)).expect("resolve root");
    assert_eq!(got_root, kb_path);
    let format_val = get_global_value_with_env("format", Some(&env))
        .expect("get format")
        .expect("format present");
    assert_eq!(format_val, "json");

    // 4. INNEN_ROOT overrides global config
    let override_kb = tempfile::tempdir().expect("tempdir");
    env.insert(
        "INNEN_ROOT".to_string(),
        override_kb.path().to_string_lossy().into_owned(),
    );
    let got_override = resolve_root_with_env(None, Some(&env)).expect("env override");
    assert_eq!(got_override, override_kb.path());

    // 5. CLI --root overrides INNEN_ROOT and global config
    let cli_kb = tempfile::tempdir().expect("tempdir");
    let got_cli =
        resolve_root_with_env(Some(cli_kb.path().to_path_buf()), Some(&env)).expect("cli");
    assert_eq!(got_cli, cli_kb.path());

    // 6. Malformed global config fails explicitly
    env.remove("INNEN_ROOT");
    let cfg_file = home.path().join("innen").join("config.json");
    fs::write(&cfg_file, "{corrupt json").expect("corrupt write");
    let corrupt_err = resolve_root_with_env(None, Some(&env)).unwrap_err();
    assert!(matches!(
        corrupt_err,
        RootResolutionError::CorruptConfig { .. }
    ));

    // 7. Relative root in global config fails explicitly
    fs::write(&cfg_file, r#"{"root": "relative/path"}"#).expect("write relative root");
    let rel_global_err = resolve_root_with_env(None, Some(&env)).unwrap_err();
    assert!(matches!(
        rel_global_err,
        RootResolutionError::RelativeGlobalRoot(_)
    ));

    // 8. Relative INNEN_ROOT fails explicitly
    env.insert("INNEN_ROOT".to_string(), "relative/kb".to_string());
    let rel_env_err = resolve_root_with_env(None, Some(&env)).unwrap_err();
    assert!(matches!(
        rel_env_err,
        RootResolutionError::RelativeEnvRoot(_)
    ));
}

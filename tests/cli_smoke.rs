use assert_cmd::Command;
use tempfile::NamedTempFile;

// Gemini is fully wired; point at an unreachable base-url so all renames
// get Transient errors → walker returns original names → identity output.
#[test]
fn gemini_offline_identity() {
    let out = NamedTempFile::new().unwrap();
    let out_path = out.path().to_owned();

    Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "gemini",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
        ])
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(contents.trim(), "const x = 1;");
}

#[test]
fn verbose_reports_resolved_config_and_rename_steps_to_stderr() {
    let out = NamedTempFile::new().unwrap();
    let out_path = out.path().to_owned();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
            "--api-key",
            "must-not-be-printed",
            "--context-size",
            "321",
            "--verbose",
        ])
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("* provider: openai"), "stderr:\n{stderr}");
    assert!(
        stderr.contains("* model: gpt-5-mini (default)"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("* base URL: http://127.0.0.1:1 (command line)"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("* API key: set (command line)"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("* context size: 321 (command line)"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("* found 1 identifiers"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("* [1/1] renaming `x`"), "stderr:\n{stderr}");
    assert!(stderr.contains("* [1/1] `x` -> `x`"), "stderr:\n{stderr}");
    assert!(!stderr.contains("must-not-be-printed"), "stderr:\n{stderr}");
}

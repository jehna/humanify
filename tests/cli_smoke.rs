use assert_cmd::Command;
use tempfile::NamedTempFile;

// TODO(task-13): switch back to openai once the openai preset is wired to a real API key in tests,
// or use a different testing approach (e.g. mock server). Using gemini stub for now.
#[test]
fn gemini_passthrough_stdin_to_file() {
    let out = NamedTempFile::new().unwrap();
    let out_path = out.path().to_owned();

    Command::cargo_bin("humanify")
        .unwrap()
        .args(["gemini", "-", "-o", out_path.to_str().unwrap()])
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(contents.trim(), "const x = 1;");
}

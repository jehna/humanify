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

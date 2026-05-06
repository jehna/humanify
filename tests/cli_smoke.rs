use assert_cmd::Command;
use tempfile::NamedTempFile;

#[test]
fn openai_passthrough_stdin_to_file() {
    let out = NamedTempFile::new().unwrap();
    let out_path = out.path().to_owned();

    Command::cargo_bin("humanify")
        .unwrap()
        .args(["openai", "-", "-o", out_path.to_str().unwrap()])
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(contents, "const x = 1;");
}

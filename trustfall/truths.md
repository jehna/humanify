# Truths

- T1: humanify is one static Rust binary, released for macOS, Linux and Windows.
  Verify: Cargo.toml defines one binary, and .github/workflows/release.yml builds it for x86_64 and aarch64 macOS and Linux, and x86_64 Windows.
- T2: Every Commit passes `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test`.
  Verify: .github/workflows/ci.yml runs all three on every pull request and push to main, and all three pass locally.
- T3: Every Preset has an end-to-end test where an LLM judge rates the Readable code of the example fixture as EXCELLENT or GOOD.
  Verify: for each Preset in src/cli/ there is a tests/e2e\_<preset>.rs that renames fixtures/example.min.js and accepts only EXCELLENT or GOOD from the judge.
- T4: End-to-end tests that spend a paid key run only on pull requests from this repository that carry the Preset's `test-<preset>` label.
  Verify: every .github/workflows/e2e-\*.yml that uses a secret other than GEMINI_API_KEY requires both the in-repository head and its `test-<preset>` label.
- T5: Unit tests read as given–when–then through the test DSL, or say in a comment why the DSL is not used.
  Verify: every `#[cfg(test)] mod tests` in src/ either uses a test_dsl helper or starts with a `DSL not applied:` comment that says why.
- T6: The project is MIT licensed.
  Verify: LICENSE holds the MIT license text, and README.md says the project is licensed under MIT.

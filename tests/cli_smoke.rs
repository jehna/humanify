use assert_cmd::Command;
use tempfile::NamedTempFile;

/// A minimal HTTP/1.1 stub standing in for an OpenAI-compatible provider.
///
/// The rest of the suite points `--base-url` at `http://127.0.0.1:1` to force
/// failures offline, which only ever exercises the error paths. Anything that
/// needs a *successful* call — a warm cache, a real `->` line, a request that
/// stalls past the run budget — needs a server, and this is the smallest one
/// that does not add a dependency.
mod stub {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    use std::sync::Arc;
    use std::time::Duration;

    enum Behavior {
        /// Answer every request with this body.
        Reply(String),
        /// Accept the connection, then never answer. Models a provider that
        /// hangs: the only thing that can end the request is a client-side
        /// deadline.
        Stall,
    }

    pub struct StubServer {
        pub base_url: String,
        addr: SocketAddr,
        requests: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
    }

    impl StubServer {
        /// A provider that always suggests `name`, in the shape
        /// `OpenAIJsonSchema` (the first rung of the default ladder) expects.
        pub fn suggesting(name: &str) -> Self {
            let body = r#"{"choices":[{"message":{"content":"{\"name\":\"NAME\"}"}}]}"#
                .replace("NAME", name);
            Self::spawn(Behavior::Reply(body))
        }

        pub fn stalling() -> Self {
            Self::spawn(Behavior::Stall)
        }

        /// Requests received so far, across all connections.
        pub fn requests(&self) -> usize {
            self.requests.load(SeqCst)
        }

        fn spawn(behavior: Behavior) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub server");
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));

            let behavior = Arc::new(behavior);
            let thread_requests = Arc::clone(&requests);
            let thread_stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if thread_stop.load(SeqCst) {
                        return;
                    }
                    let Ok(stream) = stream else { continue };
                    let behavior = Arc::clone(&behavior);
                    let requests = Arc::clone(&thread_requests);
                    // One thread per connection: a stalling handler must not
                    // wedge the accept loop.
                    std::thread::spawn(move || handle(stream, &behavior, &requests));
                }
            });

            Self {
                base_url: format!("http://{addr}/v1"),
                addr,
                requests,
                stop,
            }
        }
    }

    impl Drop for StubServer {
        fn drop(&mut self) {
            // Wake the blocking `accept` so the listener thread can observe the
            // stop flag and exit instead of outliving the test.
            self.stop.store(true, SeqCst);
            let _ = TcpStream::connect(self.addr);
        }
    }

    fn handle(mut stream: TcpStream, behavior: &Behavior, requests: &AtomicUsize) {
        let Ok(peek) = stream.try_clone() else { return };
        let mut reader = BufReader::new(peek);

        // Drain the request head, noting the body length so the body can be
        // drained too — answering before the client has finished writing gets
        // the write end reset on Windows.
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return,
                Ok(_) => {}
                Err(_) => return,
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; content_length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }

        requests.fetch_add(1, SeqCst);

        match behavior {
            Behavior::Reply(body) => {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
            Behavior::Stall => {
                // Far longer than any budget under test, and far longer than the
                // test itself: the client must be the one to give up.
                std::thread::sleep(Duration::from_secs(300));
            }
        }
    }
}

use stub::StubServer;

/// Every offline test disables retries. Against a refused connection each retry
/// is a full backoff (1s, 2s, 4s by default) for an error that will never clear,
/// which would add ~7s per identifier to the suite for no coverage.
const NO_RETRIES: [&str; 2] = ["--max-retries", "0"];

fn out_file() -> (NamedTempFile, std::path::PathBuf) {
    let f = NamedTempFile::new().unwrap();
    let p = f.path().to_owned();
    (f, p)
}

// Gemini is fully wired; point at an unreachable base-url so all renames
// get errors → walker keeps original names → identity output.
#[test]
fn gemini_offline_identity() {
    let (_out, out_path) = out_file();

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
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(contents.trim(), "const x = 1;");
}

#[test]
fn verbose_reports_resolved_config_and_rename_steps_to_stderr() {
    let server = StubServer::suggesting("counter");
    let (_out, out_path) = out_file();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            &server.base_url,
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
        stderr.contains(&format!("* base URL: {} (command line)", server.base_url)),
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
    // A real success, from a real response — not a failure laundered into
    // "returned the original name".
    assert!(
        stderr.contains("* [1/1] `x` -> `counter`"),
        "stderr:\n{stderr}"
    );
    assert!(!stderr.contains("must-not-be-printed"), "stderr:\n{stderr}");

    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(contents.trim(), "const counter = 1;");
}

#[test]
fn progress_writes_plain_snapshots_to_redirected_stderr() {
    let (_out, out_path) = out_file();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
            "--progress",
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1; const y = 2;")
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("[>-----------------------------] 0/2 identifiers"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("[==============================] 2/2 identifiers"),
        "stderr:\n{stderr}"
    );
    assert!(!stderr.contains('\r'), "stderr:\n{stderr}");
    assert!(!stderr.contains("* provider:"), "stderr:\n{stderr}");
    assert!(assert.get_output().stdout.is_empty());
}

// --- failure reporting ---

#[test]
fn failed_renames_are_reported_on_stderr() {
    let (_out, out_path) = out_file();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("`x` FAILED:"), "stderr:\n{stderr}");
    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(contents.trim(), "const x = 1;");
}

/// The regression guard for the downstream consecutive-failure counter: a failed
/// identifier must produce a `FAILED:` line and **no** `->` line. Emitting both
/// makes a failure indistinguishable from a deliberate "this name is already
/// good", and resets any caller's failure streak on every failure.
#[test]
fn a_failed_rename_emits_no_success_line() {
    let (_out, out_path) = out_file();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
            "--verbose",
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("`x` FAILED:"), "stderr:\n{stderr}");
    assert!(
        !stderr.contains("`x` -> `"),
        "a failure must not also report a rename:\n{stderr}"
    );
    assert!(
        stderr.contains("1 identifiers: 0 changed, 0 unchanged, 1 failed, 0 skipped"),
        "stderr:\n{stderr}"
    );
}

/// A failure is printed exactly once. It used to be printed twice — once by
/// `LlmRenamer` and once by the observer — which doubles stderr on a large run
/// and hands downstream parsers two messages for one event.
#[test]
fn a_failure_is_logged_once() {
    let (_out, out_path) = out_file();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let mentions = stderr
        .lines()
        .filter(|l| l.contains('`') && l.contains("x") && l.to_lowercase().contains("fail"))
        .count();
    assert_eq!(mentions, 1, "one failure, one line:\n{stderr}");
}

// --- run budget ---

#[test]
fn max_run_seconds_zero_means_unlimited() {
    let (_out, out_path) = out_file();

    Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
            "--max-run-seconds",
            "0",
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(contents.trim(), "const x = 1;");
}

/// The documented contract is "no upper bound". A week must be accepted, and so
/// must a value large enough to overflow `Instant + Duration` — that used to
/// panic rather than degrade to unlimited.
#[test]
fn very_large_budgets_are_accepted_and_not_clamped() {
    for seconds in ["604800", "18446744073709551615"] {
        let (_out, out_path) = out_file();
        Command::cargo_bin("humanify")
            .unwrap()
            .args([
                "openai",
                "-",
                "-o",
                out_path.to_str().unwrap(),
                "--base-url",
                "http://127.0.0.1:1",
                "--max-run-seconds",
                seconds,
            ])
            .args(NO_RETRIES)
            .write_stdin("const x = 1;")
            .assert()
            .success();

        let contents = std::fs::read_to_string(&out_path).unwrap();
        assert_eq!(
            contents.trim(),
            "const x = 1;",
            "--max-run-seconds {seconds}"
        );
    }
}

/// The budget is a *wall-clock* budget, so it has to bound a request that is
/// already in flight — not just the gap between identifiers. With a 600s
/// per-request timeout and a provider that never answers, only a deadline on the
/// call itself can end this run.
#[test]
fn max_run_seconds_bounds_an_in_flight_request() {
    let server = StubServer::stalling();
    let (_out, out_path) = out_file();

    let started = std::time::Instant::now();
    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .timeout(std::time::Duration::from_secs(90))
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            &server.base_url,
            "--timeout-seconds",
            "600",
            "--max-run-seconds",
            "3",
        ])
        .write_stdin("const alpha = 1; const beta = 2; const gamma = 3;")
        .assert()
        .success();
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(45),
        "run should end at the budget, not the 600s request timeout; took {elapsed:?}"
    );

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("PARTIAL:"), "stderr:\n{stderr}");

    // Exit 0 plus a complete, valid file: a partial un-minification the caller
    // can keep and resume, not a truncated one it must throw away.
    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert!(contents.contains("const alpha = 1;"), "{contents}");
    assert!(contents.contains("const beta = 2;"), "{contents}");
    assert!(contents.contains("const gamma = 3;"), "{contents}");
}

// --- cache ---

#[test]
fn cache_dir_is_created() {
    let tmp = tempfile::tempdir().unwrap();
    let (_out, out_path) = out_file();

    Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
            "--cache-dir",
            tmp.path().to_str().unwrap(),
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let cache_root = tmp.path().join("humanify-cache-v1");
    assert!(cache_root.exists(), "cache root should exist");
    // Every call failed, so nothing may have been written: caching a failure
    // would make one rate-limit blip permanent.
    let entries: Vec<_> = std::fs::read_dir(&cache_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        entries.is_empty(),
        "failed calls should not write cache entries"
    );
}

#[test]
fn no_cache_overrides_env() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_path = tmp.path().join("env_cache");
    let (_out, out_path) = out_file();

    Command::cargo_bin("humanify")
        .unwrap()
        .env("HUMANIFY_CACHE_DIR", cache_path.to_str().unwrap())
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
            "--no-cache",
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();

    assert!(
        !cache_path.exists(),
        "no-cache must prevent cache dir creation"
    );
}

/// The whole point of the cache: a second run over the same input issues no
/// requests and produces byte-identical output.
#[test]
fn warm_cache_second_run_makes_no_requests() {
    let server = StubServer::suggesting("total");
    let cache = tempfile::tempdir().unwrap();
    let source = "const a = 1; function f() { const b = 2; return b; }";

    let run = |out_path: &std::path::Path| {
        Command::cargo_bin("humanify")
            .unwrap()
            .args([
                "openai",
                "-",
                "-o",
                out_path.to_str().unwrap(),
                "--base-url",
                &server.base_url,
                "--cache-dir",
                cache.path().to_str().unwrap(),
                "--verbose",
            ])
            .write_stdin(source)
            .assert()
            .success()
            .get_output()
            .clone()
    };

    let (_o1, p1) = out_file();
    let first = run(&p1);
    let cold_requests = server.requests();
    assert!(cold_requests >= 3, "expected one call per identifier");
    let first_stderr = String::from_utf8_lossy(&first.stderr).into_owned();
    assert!(
        first_stderr.contains("0 hits"),
        "cold run should report no hits:\n{first_stderr}"
    );

    let (_o2, p2) = out_file();
    let second = run(&p2);
    assert_eq!(
        server.requests(),
        cold_requests,
        "a fully warm run must not touch the network"
    );

    let second_stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        second_stderr.contains("0 misses"),
        "warm run should report zero misses:\n{second_stderr}"
    );

    assert_eq!(
        std::fs::read_to_string(&p1).unwrap(),
        std::fs::read_to_string(&p2).unwrap(),
        "warm output must be byte-identical to cold output"
    );
}

/// The workflow `--cache-context-size` exists for: an expensive pass at a wide
/// `--context-size`, then a cheap pass at a narrow one, sharing every entry.
/// Without the flag the second run re-buys every identifier.
#[test]
fn pinned_cache_context_shares_entries_across_context_sizes() {
    let server = StubServer::suggesting("total");
    let cache = tempfile::tempdir().unwrap();
    // Big enough that the Program scope exceeds both context sizes, so the
    // truncating branches run and the windows genuinely differ.
    let mut source = String::from("const a = 1; function f() { const b = 2; return b; }");
    for _ in 0..40 {
        source.push_str("console.log(\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\");");
    }

    let run = |out_path: &std::path::Path, context_size: &str| {
        Command::cargo_bin("humanify")
            .unwrap()
            .args([
                "openai",
                "-",
                "-o",
                out_path.to_str().unwrap(),
                "--base-url",
                &server.base_url,
                "--cache-dir",
                cache.path().to_str().unwrap(),
                "--context-size",
                context_size,
                "--cache-context-size",
                "300",
                "--verbose",
            ])
            .write_stdin(source.as_str())
            .assert()
            .success()
            .get_output()
            .clone()
    };

    let (_o1, p1) = out_file();
    let first = run(&p1, "2000");
    let cold_requests = server.requests();
    assert!(cold_requests >= 3, "expected one call per identifier");

    let (_o2, p2) = out_file();
    let second = run(&p2, "300");
    assert_eq!(
        server.requests(),
        cold_requests,
        "a run at a different --context-size must still hit every pinned entry"
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("0 misses"),
        "second run should report zero misses:\n{stderr}"
    );
    // Reported on the run where the two windows actually differ. The second run
    // has them equal, so its silence is correct and is asserted below.
    let first_stderr = String::from_utf8_lossy(&first.stderr).into_owned();
    assert!(
        first_stderr.contains("* cache key context: 300 (command line) [prompt context: 2000]"),
        "a key window differing from the prompt window must be reported:\n{first_stderr}"
    );
    assert!(
        !stderr.contains("cache key context"),
        "no line is warranted when the two windows match:\n{stderr}"
    );
}

/// The control for the test above: without the flag, changing `--context-size`
/// changes the key window and the second run pays again. This is the behaviour
/// the feature exists to opt out of, so it is worth pinning down.
#[test]
fn without_the_flag_a_different_context_size_misses() {
    let server = StubServer::suggesting("total");
    let cache = tempfile::tempdir().unwrap();
    let mut source = String::from("const a = 1; function f() { const b = 2; return b; }");
    for _ in 0..40 {
        source.push_str("console.log(\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\");");
    }

    let run = |out_path: &std::path::Path, context_size: &str| {
        Command::cargo_bin("humanify")
            .unwrap()
            .args([
                "openai",
                "-",
                "-o",
                out_path.to_str().unwrap(),
                "--base-url",
                &server.base_url,
                "--cache-dir",
                cache.path().to_str().unwrap(),
                "--context-size",
                context_size,
            ])
            .write_stdin(source.as_str())
            .assert()
            .success();
    };

    let (_o1, p1) = out_file();
    run(&p1, "2000");
    let cold_requests = server.requests();

    let (_o2, p2) = out_file();
    run(&p2, "300");
    assert!(
        server.requests() > cold_requests,
        "without --cache-context-size, a different --context-size must miss"
    );
}

/// A key window with no cache to key, or an impossible one, is a silent no-op
/// that would still spend money — the same reasoning as `--refresh-cache`.
#[test]
fn cache_context_size_without_a_cache_is_rejected() {
    for extra in [
        vec!["--cache-context-size", "300"],
        vec!["--cache-context-size", "300", "--no-cache"],
    ] {
        let assert = Command::cargo_bin("humanify")
            .unwrap()
            .env_remove("HUMANIFY_CACHE_DIR")
            .env_remove("HUMANIFY_CACHE_CONTEXT_SIZE")
            .args(["openai", "-", "--base-url", "http://127.0.0.1:1"])
            .args(&extra)
            .args(NO_RETRIES)
            .write_stdin("const x = 1;")
            .assert()
            .code(64);

        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains("--cache-context-size"),
            "error should name the offending flag for {extra:?}:\n{stderr}"
        );
    }
}

/// Zero is not a window. Rejected before any input is read, so a typo costs
/// nothing rather than silently keying every identifier on an empty string.
#[test]
fn zero_cache_context_size_is_rejected() {
    let cache = tempfile::tempdir().unwrap();
    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "--base-url",
            "http://127.0.0.1:1",
            "--cache-dir",
            cache.path().to_str().unwrap(),
            "--cache-context-size",
            "0",
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .code(64);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("--cache-context-size"),
        "error should name the flag:\n{stderr}"
    );
}

/// The env var is for exporting once and forgetting, so unlike the explicit flag
/// it must not turn a `--no-cache` run into a hard error.
#[test]
fn cache_context_size_env_is_ignored_when_caching_is_off() {
    Command::cargo_bin("humanify")
        .unwrap()
        .env("HUMANIFY_CACHE_CONTEXT_SIZE", "300")
        .env_remove("HUMANIFY_CACHE_DIR")
        .args([
            "openai",
            "-",
            "--base-url",
            "http://127.0.0.1:1",
            "--no-cache",
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();
}

/// A malformed env var is a configuration mistake worth stopping for; silently
/// falling back to `--context-size` would mean a whole run at a zero hit rate.
#[test]
fn malformed_cache_context_size_env_is_rejected() {
    let cache = tempfile::tempdir().unwrap();
    Command::cargo_bin("humanify")
        .unwrap()
        .env("HUMANIFY_CACHE_CONTEXT_SIZE", "not-a-number")
        .args([
            "openai",
            "-",
            "--base-url",
            "http://127.0.0.1:1",
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .code(64);
}

/// `--refresh-cache` re-asks the model over a warm cache and the new answers
/// replace the old ones, so the following normal run serves the refreshed names.
#[test]
fn refresh_cache_re_asks_and_overwrites() {
    let cache = tempfile::tempdir().unwrap();
    let source = "const a = 1; function f() { const b = 2; return b; }";

    let run = |server: &StubServer, out_path: &std::path::Path, refresh: bool| {
        let mut cmd = Command::cargo_bin("humanify").unwrap();
        cmd.args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            &server.base_url,
            "--cache-dir",
            cache.path().to_str().unwrap(),
            "--verbose",
        ]);
        if refresh {
            cmd.arg("--refresh-cache");
        }
        cmd.write_stdin(source)
            .assert()
            .success()
            .get_output()
            .clone()
    };

    // Warm the cache with one set of names.
    let first_server = StubServer::suggesting("total");
    let (_o1, p1) = out_file();
    run(&first_server, &p1, false);
    let warm_requests = first_server.requests();
    assert!(warm_requests >= 3);

    // Refresh against a server answering differently: every identifier is
    // re-requested even though the cache is warm.
    let second_server = StubServer::suggesting("refreshed");
    let (_o2, p2) = out_file();
    let refreshed = run(&second_server, &p2, true);
    assert_eq!(
        second_server.requests(),
        warm_requests,
        "--refresh-cache must re-ask for every identifier despite a warm cache"
    );
    let stderr = String::from_utf8_lossy(&refreshed.stderr).into_owned();
    assert!(
        stderr.contains("refreshed"),
        "summary should report refreshed lookups:\n{stderr}"
    );
    assert!(
        stderr.contains("0 hits"),
        "a bypassed lookup must not be counted as a hit:\n{stderr}"
    );

    // A subsequent normal run serves the *refreshed* answers from cache.
    let third_server = StubServer::suggesting("unused");
    let (_o3, p3) = out_file();
    run(&third_server, &p3, false);
    assert_eq!(
        third_server.requests(),
        0,
        "the refreshed answers must have been written back to the cache"
    );
    assert_eq!(
        std::fs::read_to_string(&p2).unwrap(),
        std::fs::read_to_string(&p3).unwrap(),
        "the warm run after a refresh must reproduce the refreshed output"
    );
}

/// `--refresh-cache` with nothing to refresh is a silent no-op that would still
/// spend money, so it is rejected up front.
#[test]
fn refresh_cache_without_a_cache_is_rejected() {
    for extra in [
        vec!["--refresh-cache"],
        vec!["--refresh-cache", "--no-cache"],
    ] {
        let assert = Command::cargo_bin("humanify")
            .unwrap()
            .env_remove("HUMANIFY_CACHE_DIR")
            .args(["openai", "-", "--base-url", "http://127.0.0.1:1"])
            .args(&extra)
            .args(NO_RETRIES)
            .write_stdin("const x = 1;")
            .assert()
            .code(64);

        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains("--refresh-cache"),
            "error should name the offending flag for {extra:?}:\n{stderr}"
        );
    }
}

/// A secret must never influence a filename or land in a cache entry — the
/// directory is meant to be shareable and long-lived.
#[test]
fn api_key_never_reaches_the_cache() {
    const SECRET: &str = "sk-do-not-persist-me-0123456789";
    let server = StubServer::suggesting("total");
    let cache = tempfile::tempdir().unwrap();
    let (_out, out_path) = out_file();

    Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            &server.base_url,
            "--api-key",
            SECRET,
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .write_stdin("const a = 1;")
        .assert()
        .success();

    let mut checked = 0usize;
    let mut stack = vec![cache.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
            let path = entry.path();
            assert!(
                !path.to_string_lossy().contains(SECRET),
                "API key leaked into a path: {}",
                path.display()
            );
            if path.is_dir() {
                stack.push(path);
            } else {
                let body = std::fs::read_to_string(&path).unwrap_or_default();
                assert!(
                    !body.contains(SECRET),
                    "API key leaked into {}",
                    path.display()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "expected at least one cache entry to inspect");
}

/// A cache that cannot be opened is a warning, never a failed run: the cache
/// exists to protect a long run, so it must not be the thing that kills one.
#[test]
fn unwritable_cache_dir_still_produces_output() {
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join("i_am_a_file");
    std::fs::write(&blocker, "not a directory").unwrap();
    let unusable = blocker.join("cache");
    let (_out, out_path) = out_file();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:1",
            "--cache-dir",
            unusable.to_str().unwrap(),
        ])
        .args(NO_RETRIES)
        .write_stdin("const x = 1;")
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("cache disabled"), "stderr:\n{stderr}");
    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(contents.trim(), "const x = 1;");
}

// --- sentinel-bounded renaming ---

/// A helper (`helperFn`) declared before the marked region and called inside it,
/// a local in the region, and a function after it that the region never touches.
const SENTINEL_SOURCE: &str = concat!(
    "function helperFn(input) { return input + 1; }\n",
    "function afterFn() { const late = 3; return late; }\n",
    "function target() { const local = helperFn(2); return local; }\n",
);

/// Only the identifiers inside the window are bought, and the ones outside keep
/// their original names — the whole point of the feature.
#[test]
fn a_sentinel_window_limits_which_identifiers_are_bought() {
    let server = StubServer::suggesting("renamed");
    let (_out, out_path) = out_file();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            &server.base_url,
            "--start-sentinel",
            "function target",
            "--stop-sentinel",
            "return local; }",
        ])
        .write_stdin(SENTINEL_SOURCE)
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("sentinel window: bytes"),
        "the resolved window is reported without --verbose:\n{stderr}"
    );
    // target + local + helperFn (called inside the window). Not afterFn, not
    // `late`, not `input`.
    assert!(
        stderr.contains("3 of 6 identifiers selected"),
        "stderr:\n{stderr}"
    );
    assert_eq!(server.requests(), 3, "one call per selected identifier");

    let contents = std::fs::read_to_string(&out_path).unwrap();
    assert!(
        contents.contains("function afterFn"),
        "an out-of-window function keeps its name:\n{contents}"
    );
    assert!(
        contents.contains("late"),
        "an out-of-window local keeps its name:\n{contents}"
    );
    assert!(
        !contents.contains("helperFn"),
        "a helper called inside the window is renamed:\n{contents}"
    );
}

#[test]
fn dry_run_prints_the_selection_and_makes_zero_requests() {
    let server = StubServer::suggesting("renamed");
    let (_out, out_path) = out_file();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            out_path.to_str().unwrap(),
            "--base-url",
            &server.base_url,
            "--start-sentinel",
            "function target",
            "--stop-sentinel",
            "return local; }",
            "--dry-run",
        ])
        .write_stdin(SENTINEL_SOURCE)
        .assert()
        .success();

    assert_eq!(server.requests(), 0, "a dry run must not call the provider");

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("dry run"), "stderr:\n{stderr}");
    assert!(
        stderr.contains("sentinel window: bytes"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("selected identifiers: helperFn, local, target"),
        "stderr:\n{stderr}"
    );

    // `-o` was given, and a dry run still leaves it untouched.
    assert_eq!(std::fs::read_to_string(&out_path).unwrap(), "");
}

#[test]
fn an_ambiguous_fragment_fails_with_64_listing_every_occurrence() {
    let server = StubServer::suggesting("renamed");

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "--base-url",
            &server.base_url,
            "--start-sentinel",
            "function ",
        ])
        .write_stdin(SENTINEL_SOURCE)
        .assert()
        .code(64);

    assert_eq!(server.requests(), 0, "nothing is bought before resolution");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("matches 3 times"), "stderr:\n{stderr}");
    assert!(stderr.contains("1:1"), "stderr:\n{stderr}");
    assert!(stderr.contains("2:1"), "stderr:\n{stderr}");
    assert!(stderr.contains("3:1"), "stderr:\n{stderr}");
}

#[test]
fn a_sentinel_fragment_reads_from_an_at_file() {
    let server = StubServer::suggesting("renamed");
    let dir = tempfile::tempdir().unwrap();
    let fragment = dir.path().join("start.txt");
    // Trailing newline as an editor would leave it; quoting this on a shell
    // command line is exactly what `@file` exists to avoid.
    std::fs::write(&fragment, "const local = helperFn(2);\n").unwrap();

    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "--base-url",
            &server.base_url,
            "--start-sentinel",
            &format!("@{}", fragment.display()),
            "--dry-run",
        ])
        .write_stdin(SENTINEL_SOURCE)
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("selected identifiers: helperFn, local"),
        "stderr:\n{stderr}"
    );
}

/// The cache key covers the question only, so a sentinel run and a later
/// whole-file run share answers: buy the region with an expensive model, then
/// sweep the rest cheaply without re-buying it.
#[test]
fn a_sentinel_run_warms_the_cache_for_a_later_whole_file_run() {
    let server = StubServer::suggesting("renamed");
    let cache = tempfile::tempdir().unwrap();

    let (_o1, p1) = out_file();
    Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            p1.to_str().unwrap(),
            "--base-url",
            &server.base_url,
            "--cache-dir",
            cache.path().to_str().unwrap(),
            "--start-sentinel",
            "function target",
            "--stop-sentinel",
            "return local; }",
        ])
        .write_stdin(SENTINEL_SOURCE)
        .assert()
        .success();
    let after_region = server.requests();
    assert_eq!(after_region, 3);

    let (_o2, p2) = out_file();
    let assert = Command::cargo_bin("humanify")
        .unwrap()
        .args([
            "openai",
            "-",
            "-o",
            p2.to_str().unwrap(),
            "--base-url",
            &server.base_url,
            "--cache-dir",
            cache.path().to_str().unwrap(),
            "--verbose",
        ])
        .write_stdin(SENTINEL_SOURCE)
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("3 hits"),
        "the region's answers should be reused verbatim:\n{stderr}"
    );
    assert_eq!(
        server.requests(),
        after_region + 3,
        "only the three identifiers outside the region are bought"
    );
}

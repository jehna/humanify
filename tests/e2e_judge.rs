//! Sanity checks for the local-Ollama judge prompt: each verdict label must
//! be reachable from a representative snippet that is *clearly* in that class.
//! These guard the few-shot calibration so a regressed prompt fails fast.

mod common;

use common::judge;

async fn assert_verdict(code: &str, expected: &str) {
    let verdict = judge(code).await.expect("judge failed");
    assert_eq!(
        verdict, expected,
        "expected {expected} for snippet:\n{code}\ngot {verdict}"
    );
}

// Snippets below are intentionally distinct from the few-shot examples in
// JUDGE_SYSTEM so the tests measure generalization, not memorization.

#[tokio::test]
#[ignore]
async fn judge_calls_minified_minified() {
    let code = "var x=function(o,n){return o.filter(function(e){return e>n})};var y=x([1,2,3,4],2);";
    assert_verdict(code, "MINIFIED").await;
}

#[tokio::test]
#[ignore]
async fn judge_calls_gibberish_gibberish() {
    // A function that filters an array greater than a threshold, but named and
    // wired up as if it were doing HTTP/date work. The shape we want the judge
    // to flag as a hallucinated rename.
    let code = "function fetchUserProfile(items, minScore) {\n    return items.filter(function(banana) { return banana > minScore; });\n}\nconst yesterdaysWeather = fetchUserProfile([1, 2, 3, 4], 2);";
    assert_verdict(code, "GIBBERISH").await;
}

#[tokio::test]
#[ignore]
async fn judge_calls_excellent_excellent() {
    let code = "function filterAboveThreshold(values, threshold) {\n    return values.filter(function(value) { return value > threshold; });\n}\nconst aboveTwo = filterAboveThreshold([1, 2, 3, 4], 2);";
    assert_verdict(code, "EXCELLENT").await;
}

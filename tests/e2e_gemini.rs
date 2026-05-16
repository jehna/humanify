mod common;

use common::{given, humanify, JudgeConfig};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_gemini() {
    given("fixtures/example.min.js")
        .judged_by(JudgeConfig::gemini("gemini-3.1-flash-lite"))
        .judge_says_minified()
        .await
        .when(humanify().gemini().model("gemini-3.1-flash-lite"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

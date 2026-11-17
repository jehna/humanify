mod common;

use common::{given, humanify, JudgeConfig};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_cheaperinference() {
    given("fixtures/example.min.js")
        .judged_by(JudgeConfig::cheaperinference("gpt-5.4-mini"))
        .judge_says_minified()
        .await
        .when(humanify().cheaperinference().model("gpt-5.4-mini"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

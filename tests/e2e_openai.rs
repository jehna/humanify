mod common;

use common::{given, humanify, JudgeConfig};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_openai() {
    given("fixtures/example.min.js")
        .judged_by(JudgeConfig::openai("gpt-5-mini"))
        .judge_says_minified()
        .await
        .when(humanify().openai().model("gpt-5-mini"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

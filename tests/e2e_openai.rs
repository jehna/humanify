mod common;

use common::{given, humanify, JudgeConfig};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_openai() {
    given("fixtures/example.min.js")
        .judged_by(JudgeConfig::openai("gpt-6.1-terra"))
        .judge_says_minified()
        .await
        .when(humanify().openai().model("gpt-6.1-terra"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

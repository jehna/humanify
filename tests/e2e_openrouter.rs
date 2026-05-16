mod common;

use common::{given, humanify, JudgeConfig};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_openrouter() {
    given("fixtures/example.min.js")
        .judged_by(JudgeConfig::openrouter("openai/gpt-oss-120b"))
        .judge_says_minified()
        .await
        .when(humanify().openrouter().model("openai/gpt-oss-120b"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

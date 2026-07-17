mod common;

use common::{given, humanify, JudgeConfig};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_requesty() {
    given("fixtures/example.min.js")
        .judged_by(JudgeConfig::requesty("openai/gpt-4o-mini"))
        .judge_says_minified()
        .await
        .when(humanify().requesty().model("openai/gpt-4o-mini"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

mod common;

use common::{given, humanify, JudgeConfig};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_anthropic() {
    given("fixtures/example.min.js")
        .judged_by(JudgeConfig::anthropic("claude-sonnet-4-6"))
        .judge_says_minified()
        .await
        .when(humanify().anthropic().model("claude-sonnet-4-6"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

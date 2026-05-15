mod common;

use common::{given, humanify};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_openrouter() {
    given("fixtures/example.min.js")
        .judge_says_minified()
        .await
        .when(humanify().openrouter().model("qwen/qwen3-coder:free"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

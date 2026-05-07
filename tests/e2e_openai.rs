mod common;

use common::{given, humanify};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_openai() {
    given("fixtures/example.min.js")
        .judge_says_unreadable()
        .await
        .when(humanify().openai().model("gpt-5.4-mini"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

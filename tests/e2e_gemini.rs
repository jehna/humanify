mod common;

use common::{given, humanify};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_gemini() {
    given("fixtures/example.min.js")
        .judge_says_unreadable()
        .await
        .when(humanify().gemini().model("gemini-3.1-flash-lite"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

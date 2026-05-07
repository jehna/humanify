mod common;

use common::{given, humanify};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_anthropic() {
    given("fixtures/example.min.js")
        .judge_says_unreadable()
        .await
        .when(humanify().anthropic().model("claude-sonnet-4-6"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

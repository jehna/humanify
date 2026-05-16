mod common;

use common::{given, humanify};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_ollama() {
    given("fixtures/example.min.js")
        .judge_says_minified()
        .await
        .when(humanify().ollama().model("gemma3:4b"))
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

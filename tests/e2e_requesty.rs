mod common;

use common::{given, humanify, JudgeConfig};

#[tokio::test]
#[ignore]
async fn unminifies_example_file_with_requesty() {
    given("fixtures/example.min.js")
        .judged_by(JudgeConfig::requesty("nvidia/nemotron-3-super-120b-a12b"))
        .judge_says_minified()
        .await
        .when(
            humanify()
                .requesty()
                .model("nvidia/nemotron-3-super-120b-a12b"),
        )
        .await
        .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
        .await;
}

import test from "node:test";
import { readFile } from "node:fs/promises";
import { testPrompt } from "./test-prompt.js";
import { gbnf } from "../plugins/local-llm-rename/gbnf.js";
import assert from "node:assert";
import { humanify } from "../test-utils.js";

const TEST_OUTPUT_DIR = "test-output";

test.afterEach(async () => {
  //await rm(TEST_OUTPUT_DIR, { recursive: true, force: true });
});

test("Unminifies an example file successfully", async () => {
  await humanify(
    "local",
    "fixtures/example.min.js",
    "--verbose",
    "--outputDir",
    TEST_OUTPUT_DIR
  );
  // For debugging:
  /*await cp(
    "fixtures/example.min.js",
    `${TEST_OUTPUT_DIR}/deobfuscated.js`
  );*/
  const result = await readFile(`${TEST_OUTPUT_DIR}/deobfuscated.js`, "utf-8");
  const prompt = await testPrompt();
  assert.equal(
    await prompt(
      `Does this code look easy to read? Answer "YES" or "NO"`,
      result,
      gbnf`${/("YES" | "NO")/}, this code is`
    ),
    "YES"
  );
});

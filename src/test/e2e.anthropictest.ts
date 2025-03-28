import test from "node:test";
import { readFile, rm } from "node:fs/promises";
import { testPrompt } from "./test-prompt.js";
import { gbnf } from "../plugins/local-llm-rename/gbnf.js";
import assert from "node:assert";
import { humanify } from "../test-utils.js";

const TEST_OUTPUT_DIR = "test-output";

test.afterEach(async () => {
  await rm(TEST_OUTPUT_DIR, { recursive: true, force: true });
});

test("Unminifies an example file successfully", async () => {
  const fileIsMinified = async (filename: string) => {
    const prompt = await testPrompt();
    return await prompt(
      `Your job is to read the following code and rate it's readabillity and variable names. Answer "EXCELLENT", "GOOD" or "UNREADABLE".`,
      await readFile(filename, "utf-8"),
      gbnf`${/("EXCELLENT" | "GOOD" | "UNREADABLE") [^.]+/}.`
    );
  };
  const expectStartsWith = (expected: string[], actual: string) => {
    assert(
      expected.some((e) => actual.startsWith(e)),
      `Expected "${expected}" but got ${actual}`
    );
  };

  await expectStartsWith(
    ["UNREADABLE"],
    await fileIsMinified(`fixtures/example.min.js`)
  );

  await humanify(
    "anthropic",
    "fixtures/example.min.js",
    "--verbose",
    "--outputDir",
    TEST_OUTPUT_DIR
  );

  await expectStartsWith(
    ["EXCELLENT", "GOOD"],
    await fileIsMinified(`${TEST_OUTPUT_DIR}/deobfuscated.js`)
  );
});

import test from "node:test";
import { readFile, rm } from "node:fs/promises";
import assert from "node:assert";
import { humanify } from "../test-utils.js";
import { Ollama } from "ollama";

const TEST_OUTPUT_DIR = "test-output";
const TEST_MODEL = "gpt-oss:20b";

test.afterEach(async () => {
  await rm(TEST_OUTPUT_DIR, { recursive: true, force: true });
});

test("Unminifies an example file successfully", async () => {
  const ollama = new Ollama({ host: "http://localhost:11434" });

  const fileIsMinified = async (filename: string) => {
    const code = await readFile(filename, "utf-8");
    const response = await ollama.chat({
      model: TEST_MODEL,
      messages: [
        {
          role: "user",
          content: `Read the following code and rate its readability and variable names. Answer only with one word: "EXCELLENT", "GOOD", or "UNREADABLE".\n\nCode:\n${code}`
        }
      ]
    });
    return response.message.content.trim();
  };

  const expectStartsWith = (expected: string[], actual: string) => {
    assert(
      expected.some((e) => actual.toUpperCase().startsWith(e)),
      `Expected one of "${expected}" but got "${actual}"`
    );
  };

  // Check the minified file readability (might be GOOD or UNREADABLE depending on model)
  const minifiedReadability = await fileIsMinified(`fixtures/example.min.js`);
  console.log(`Minified file rated as: ${minifiedReadability}`);

  await humanify(
    "ollama",
    "fixtures/example.min.js",
    "-m",
    TEST_MODEL,
    "--verbose",
    "--outputDir",
    TEST_OUTPUT_DIR
  );

  const deobfuscatedReadability = await fileIsMinified(`${TEST_OUTPUT_DIR}/deobfuscated.js`);
  console.log(`Deobfuscated file rated as: ${deobfuscatedReadability}`);

  // The deobfuscated file should be rated EXCELLENT or GOOD
  await expectStartsWith(
    ["EXCELLENT", "GOOD"],
    deobfuscatedReadability
  );
});

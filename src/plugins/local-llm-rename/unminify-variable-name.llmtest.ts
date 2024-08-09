import test from "node:test";
import { unminifyVariableName } from "./unminify-variable-name.js";
import { assertMatches } from "../../test-utils.js";
import { testPrompt } from "../../test/test-prompt.js";

const prompt = await testPrompt();

test("Unminifies a function name", async () => {
  const result = await unminifyVariableName(
    prompt,
    "a",
    "math-utils.js",
    "const a = b => b + 1;"
  );
  assertMatches(result, ["increment", "addOne"]);
});

test("Unminifies an argument", async () => {
  const result = await unminifyVariableName(
    prompt,
    "b",
    "math-utils.js",
    "const addOne = b => b + 1;"
  );
  assertMatches(result, ["num", "val", "accumulator", "increment"]);
});

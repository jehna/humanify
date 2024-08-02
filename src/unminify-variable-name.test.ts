import test from "node:test";
import { llama } from "./llama.js";
import { unminifyVariableName } from "./unminify-variable-name.js";
import { assertMatches } from "./test-utils.js";
import { DEFAULT_MODEL, getEnsuredModelPath } from "./local-models.js";

const prompt = await llama({
  seed: 1,
  modelPath: getEnsuredModelPath(process.env["MODEL"] ?? DEFAULT_MODEL)
});

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
  assertMatches(result, ["num", "number", "val", "value", "accumulator"]);
});

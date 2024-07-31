import test from "node:test";
import { llama } from "./llama.js";
import { unminifyVariableName } from "./unminify-variable-name.js";
import { assertMatches } from "./test-utils.js";

const prompt = await llama({ seed: 1, modelPath: process.env["MODEL"] }); // TODO: Figure out how to download a small enough model

test("Unminifies a function name", async (t) => {
  const result = await unminifyVariableName(
    prompt,
    "a",
    "math-utils.js",
    "const a = b => b + 1;"
  );
  assertMatches(result, ["increment", "addOne"]);
});

test("Unminifies an argument", async (t) => {
  const result = await unminifyVariableName(
    prompt,
    "b",
    "math-utils.js",
    "const addOne = b => b + 1;"
  );
  assertMatches(result, ["num", "number", "val", "value", "accumulator"]);
});

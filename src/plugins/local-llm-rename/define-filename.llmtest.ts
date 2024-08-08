import test from "node:test";
import { llama } from "./llama.js";
import { assertMatches } from "../../test-utils.js";
import { DEFAULT_MODEL, getEnsuredModelPath } from "../../local-models.js";
import { defineFilename } from "./define-filename.js";

const prompt = await llama({
  seed: 1,
  modelPath: getEnsuredModelPath(process.env["MODEL"] ?? DEFAULT_MODEL)
});

test("Defines a good name for a file with a function", async () => {
  const result = await defineFilename(prompt, "const a = b => b + 1;");
  assertMatches(result, ["increment.js", "addOne.js"]);
});

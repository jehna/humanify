import { DEFAULT_MODEL, getEnsuredModelPath } from "../local-models.js";
import { llama } from "../plugins/local-llm-rename/llama.js";

export const testPrompt = async () =>
  await llama({
    seed: 1,
    modelPath: getEnsuredModelPath(process.env["MODEL"] ?? DEFAULT_MODEL)
  });

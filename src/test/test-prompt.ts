import { DEFAULT_MODEL } from "../local-models.js";
import { llama } from "../plugins/local-llm-rename/llama.js";

export const testPrompt = async () =>
  await llama({
    seed: 1,
    model: process.env["MODEL"] ?? DEFAULT_MODEL
  });

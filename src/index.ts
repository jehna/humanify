import { readFile } from "fs/promises";
import { llama } from "./llama.js";
import { unminifyVariableName } from "./unminify-variable-name.js";

console.log(
  await unminifyVariableName(
    await llama({ modelPath: "models/Phi-3.1-mini-4k-instruct-Q4_K_M.gguf" }),
    "a",
    "string-utils.js",
    await readFile("example.min.js", "utf-8")
  )
);

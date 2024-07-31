import { readFile } from "fs/promises";
import { llama } from "./llama.js";
import { unminifyVariableName } from "./unminify-variable-name.js";

console.log(
  await unminifyVariableName(
    await llama(),
    "a",
    "string-utils.js",
    await readFile("example.min.js", "utf-8")
  )
);

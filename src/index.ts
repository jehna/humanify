import { readFile } from "fs/promises";
import { llama } from "./llama.js";
import { gbnf } from "./gbnf.js";

const prompt = await llama();

const filename = "string-utils.js";

const description = await prompt(
  `Your task is to read the code in file "${filename}" and write the purpose of each variable and function in one sentence.`,
  await readFile("example.min.js", "utf-8"),
  gbnf`'a' is ${/[^\r\n\x0b\x0c\x85\u2028\u2029.]+/}.`
);
console.log(description);

for (let i = 0; i < 10; i++)
  console.log(
    await prompt(
      `You are a Code Assistant.`,
      `What would be a good name for the following function or a variable in Typescript? Don't mind the minified variable names.\n${description}`,
      gbnf`A good name would be '${/[a-zA-Z] [a-zA-Z0-9]{2,12}/}'`
    )
  );

import { verbose } from "../../verbose.js";
import { gbnf } from "./gbnf.js";
import { Prompt } from "./llama.js";

export async function defineFilename(prompt: Prompt, code: string) {
  verbose.log("Defining filename for code snippet:\n", code);
  const description = await prompt(
    `What does this code do? Answer in one short sentence starting with a verb (e.g., "Adds", "Calculates", "Returns"). Do not mention variable names.`,
    code,
    gbnf`${/[^\r\n\x0b\x0c\x85\u2028\u2029.]+/}.`
  );
  verbose.log("Description:", description);

  const filename = await prompt(
    `Suggest a short camelCase filename (without extension) for a JavaScript file. The filename should be a single word or two words combined, like "increment" or "addOne". Description:`,
    description,
    gbnf`A good filename would be '${/[a-z] [a-zA-Z0-9]{2,12}/}'`
  );

  return `${filename}.js`;
}

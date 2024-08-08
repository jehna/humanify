import { gbnf } from "./gbnf.js";
import { Prompt } from "./llama.js";

export async function defineFilename(prompt: Prompt, code: string) {
  const description = await prompt(
    `Simplify the code snippet's purpose into a concise explanation in one sentence. Don't use variable names or function names in your description. Use the present tense.`,
    code,
    gbnf`${/[^\r\n\x0b\x0c\x85\u2028\u2029.]+/}.`
  );
  //console.log(description);

  const filename = await prompt(
    `Create a name for a Javascript file for a code with the following description. Use lisp-case naming convention.`,
    description,
    gbnf`Sure, a good name for your Javascript file would be '${/[a-z] [a-zA-Z0-9]{2,12}/}.js'`
  );

  return `${filename}.js`;
}

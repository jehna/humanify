import { gbnf } from "./gbnf.js";
import { Prompt } from "./llama.js";

export async function unminifyVariableName(
  prompt: Prompt,
  variableName: string,
  filename: string,
  code: string
) {
  const description = await prompt(
    `Your task is to read the code in file "${filename}" and write the purpose of variable, argument or function '${variableName}' in one sentence. Use simple language so it's understandable by a junior programmer.`,
    code,
    gbnf`A good description for '${variableName}' is: ${/[^\r\n\x0b\x0c\x85\u2028\u2029.]+/}.`
  );
  //console.log(description);

  return await prompt(
    `You are a Code Assistant.`,
    `What would be a good name for the following function or a variable in Typescript? Don't mind the minified variable names.\n${description}`,
    gbnf`A good name would be '${/[a-zA-Z] [a-zA-Z0-9]{2,12}/}'`
  );
}

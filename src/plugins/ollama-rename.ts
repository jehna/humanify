import ollama, { ChatRequest } from "ollama";
import { verbose } from "../verbose.js";
import { visitAllIdentifiers } from "./local-llm-rename/visit-all-identifiers.js";
import { showPercentage } from "../progress.js";
import { gbnf } from "./local-llm-rename/gbnf.js";


export function ollamaRename({
  model
}: {
  model: string;
}) {
  return async (code: string): Promise<string> => {
    return await visitAllIdentifiers(
      code,
      async (name, surroundingCode) => {
        verbose.log(`Renaming ${name}`);
        verbose.log("Context: ", surroundingCode);

        const response = await ollama.chat(
          toRenamePrompt(name, surroundingCode, model)
        );
        const result = response.message?.content;
        if (!result) {
          throw new Error("Failed to rename", { cause: response });
        }
        const renamed = JSON.parse(result).newName;

        verbose.log(`Renamed to ${renamed}`);

        return renamed;
      },
      showPercentage
    );
  };
}

function toRenamePrompt(
  name: string,
  surroundingCode: string,
  model: string
): ChatRequest & { stream?: false } {
  return {
    model,
    messages: [
      {
        role: "system",
        content: `Rename Javascript variables/function \`${name}\` to have descriptive name based on their usage in the code."`
      },
      {
        role: "user",
        content: surroundingCode
      }
    ],
    stream: false,
    options: ({
      grammar: gbnf`Sure! A good name for variable ${name} would be ${/[a-zA-Z_][a-zA-Z0-9_]*/}`
    } as any)
  };
}

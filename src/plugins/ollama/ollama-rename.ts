import { Ollama } from "ollama";
import { visitAllIdentifiers } from "../local-llm-rename/visit-all-identifiers.js";
import { showPercentage } from "../../progress.js";
import { verbose } from "../../verbose.js";

export function ollamaRename({
  baseURL,
  model,
  contextWindowSize
}: {
  baseURL?: string;
  model: string;
  contextWindowSize: number;
}) {
  const client = new Ollama({ host: baseURL });

  return async (code: string): Promise<string> => {
    return await visitAllIdentifiers(
      code,
      async (name, surroundingCode) => {
        verbose.log(`Renaming ${name}`);
        verbose.log("Context: ", surroundingCode);

        const response = await client.chat({
          model,
          messages: [
            {
              role: "system",
              content: `Rename Javascript variables/function \`${name}\` to have descriptive name based on their usage in the code. Respond only with valid JSON in the format: {"newName": "your_new_name_here"}`
            },
            {
              role: "user",
              content: surroundingCode
            }
          ],
          format: "json"
        });

        const result = response.message?.content;
        if (!result) {
          throw new Error("Failed to rename", { cause: response });
        }
        const renamed = JSON.parse(result).newName;

        verbose.log(`Renamed to ${renamed}`);

        return renamed;
      },
      contextWindowSize,
      showPercentage
    );
  };
}

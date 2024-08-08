import OpenAI from "openai";
import { visitAllIdentifiers } from "../local-llm-rename/visit-all-identifiers.js";
import { showPercentage } from "../../progress.js";

export function openaiRename({
  apiKey,
  model
}: {
  apiKey: string;
  model: string;
}) {
  const client = new OpenAI({ apiKey });
  return async (code: string): Promise<string> => {
    return await visitAllIdentifiers(
      code,
      async (name, surroundingCode) => {
        const response = await client.chat.completions.create(
          toRenamePrompt(name, surroundingCode, model)
        );
        const result = response.choices[0].message?.content;
        if (!result) {
          throw new Error("Failed to rename", { cause: response });
        }
        return JSON.parse(result).newName;
      },
      showPercentage
    );
  };
}

function toRenamePrompt(
  name: string,
  surroundingCode: string,
  model: string
): OpenAI.Chat.Completions.ChatCompletionCreateParamsNonStreaming {
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
    response_format: {
      type: "json_schema",
      json_schema: {
        strict: true,
        name: "rename",
        schema: {
          type: "object",
          properties: {
            newName: {
              type: "string",
              description: `The new name for the variable/function called \`${name}\``
            }
          },
          required: ["newName"],
          additionalProperties: false
        }
      }
    }
  };
}

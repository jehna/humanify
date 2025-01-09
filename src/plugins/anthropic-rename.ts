import Anthropic from "@anthropic-ai/sdk";
import { visitAllIdentifiers } from "./local-llm-rename/visit-all-identifiers.js";
import { showPercentage } from "../progress.js";
import { verbose } from "../verbose.js";

export function anthropicRename({
  apiKey,
  baseURL,
  model,
  contextWindowSize
}: {
  apiKey: string;
  baseURL?: string;
  model: string;
  contextWindowSize: number;
}) {
  const client = new Anthropic({
    apiKey,
    baseURL
  });

  return async (code: string): Promise<string> => {
    return await visitAllIdentifiers(
      code,
      async (name, surroundingCode) => {
        verbose.log(`Renaming ${name}`);
        verbose.log("Context: ", surroundingCode);

        const response = await client.messages.create(
          toRenamePrompt(name, surroundingCode, model, contextWindowSize)
        );

        const result = response.content[0];
        if (!result) {
          throw new Error('Failed to rename', { cause: response });
        }
        const renamed = result.input.newName
        verbose.log(`${name} renamed to ${renamed}`);
        return renamed;
      },
      contextWindowSize,
      showPercentage
    );
  };
}

function toRenamePrompt(
  name: string,
  surroundingCode: string,
  model: string,
  contextWindowSize: number,
): Anthropic.Messages.MessageCreateParams {
  return {
    model,
    messages: [
      {
        role: "user",
        content: `Analyze this code and suggest a descriptive name for the variable/function \`${name}\`:
        ${surroundingCode}`
      }
    ],
    max_tokens: contextWindowSize,
    tools: [
      {
        name: "suggest_name",
        description: "Suggest a descriptive name for the code element",
        input_schema: {
          type: "object",
          properties: {
            newName: {
              type: "string",
              description: `The new descriptive name for the variable/function called \`${name}\``
            }
          },
          required: ["newName"],
          additionalProperties: false
        }
      }
    ],
    tool_choice: {
      type: "tool",
      name: "suggest_name"
    }
  };
}
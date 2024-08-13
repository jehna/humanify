import ModelClient, { ChatCompletionsFunctionToolCallOutput, GetChatCompletionsParameters } from "@azure-rest/ai-inference";
import { AzureKeyCredential } from "@azure/core-auth";
import { visitAllIdentifiers } from "./local-llm-rename/visit-all-identifiers.js";
import { verbose } from "../verbose.js";
import { showPercentage } from "../progress.js";

export function azureRename({
  apiKey,
  model
}: {
  apiKey: string;
  model: string;
}) {
  const client = ModelClient("https://models.inference.ai.azure.com", new AzureKeyCredential(apiKey));

  return async (code: string): Promise<string> => {
    return await visitAllIdentifiers(
      code,
      async (name, surroundingCode) => {
        verbose.log(`Renaming ${name}`);
        verbose.log("Context: ", surroundingCode);

        const response = await client.path('/chat/completions').post(
          toRenamePrompt(name, surroundingCode, model)
        );

        if ("error" in response.body) {
          throw new Error("Failed to rename", { cause: response.body.error });
        }

        const toolCall = response.body.choices[0].message.tool_calls?.[0];
        if (!toolCall || toolCall.type !== "function") {
          throw new Error("Failed to rename", { cause: toolCall });
        }
        const result = (toolCall as unknown as ChatCompletionsFunctionToolCallOutput).function.arguments
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
): GetChatCompletionsParameters {
  return {
    body: {
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
      tools: [
        {
          type: "function",
          function: {
            description: "Rename Javascript variables/function to have descriptive name based on their usage in the code.",
            name: "rename",
            parameters: {
              "description": "",
              "type": "object",
              "properties": {
                "newName": {
                  "type": "string",
                  "minLength": 3
                }
              },
              "required": [
                "newName"
              ]
            }
          }
        }
      ],
      tool_choice: {function: { name: "rename"}, type: "function"},
    }
  };
}

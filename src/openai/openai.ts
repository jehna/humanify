import { Configuration, OpenAIApi } from "openai";
import { splitCode } from "./split-file.js";
import {
  Rename,
  renameVariablesAndFunctions,
} from "./rename-variables-and-functions.js";

type Options = {
  apiKey: string;
  use4k: boolean;
};

export default ({ apiKey, use4k }: Options) => {
  const client = new OpenAIApi(new Configuration({ apiKey }));

  return async (code: string): Promise<string> => {
    const codeBlocks = await splitCode(code, use4k);
    let variablesAndFunctionsToRename: Rename[] = [];
    for (const codeBlock of codeBlocks) {
      const renames = await codeToVariableRenames(codeBlock);
      variablesAndFunctionsToRename =
        variablesAndFunctionsToRename.concat(renames);
    }
    return renameVariablesAndFunctions(code, variablesAndFunctionsToRename);
  };

  async function codeToVariableRenames(code: string) {
    const chatCompletion = await client.createChatCompletion({
      model: use4k ? "gpt-3.5-turbo" : "gpt-3.5-turbo-16k",
      functions: [
        {
          name: "rename_variables_and_functions",
          description: "Rename variables and function names in Javascript code",
          parameters: {
            type: "object",
            properties: {
              variablesAndFunctionsToRename: {
                type: "array",
                items: {
                  type: "object",
                  properties: {
                    name: {
                      type: "string",
                      description:
                        "The name of the variable or function name to rename",
                    },
                    newName: {
                      type: "string",
                      description:
                        "The new name of the variable or function name",
                    },
                  },
                  required: ["name", "newName"],
                },
              },
            },
            required: ["variablesToRename"],
          },
        },
      ],
      messages: [
        {
          role: "assistant",
          content:
            "Rename all Javascript variables and functions to have descriptive names based on their usage in the code.",
        },
        { role: "user", content: code },
      ],
    });
    const data = chatCompletion.data.choices[0];
    if (data.finish_reason !== "function_call") return [];

    const {
      variablesAndFunctionsToRename,
    }: { variablesAndFunctionsToRename: Rename[] } = JSON.parse(
      fixPerhapsBrokenResponse(data.message?.function_call?.arguments!)
    );

    return variablesAndFunctionsToRename;
  }
};

function fixPerhapsBrokenResponse(jsonResponse: string) {
  // Sometimes the response has an extra comma at the end of the array, like:
  // {"result": [{"foo": "bar"}, { "foo": "baz" },\n ]}
  // This is invalid JSON, so we need to remove the comma.

  return jsonResponse.replace(/},\s*]/im, "}]");
}

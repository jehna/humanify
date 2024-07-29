import {  OpenAI } from "openai";
import { splitCode } from "./split-file.js";
import {
  Rename,
  renameVariablesAndFunctions,
} from "./rename-variables-and-functions.js";
import { mapPromisesParallel } from "./run-promises-in-parallel.js";

type Options = {
  apiKey: string;
};

export default ({ apiKey }: Options) => {
  const openai = new OpenAI({ apiKey: apiKey, });

  return async (code: string): Promise<string> => {
    const codeBlocks = await splitCode(code);
    let variablesAndFunctionsToRename: Rename[] = [];
    await mapPromisesParallel(10, codeBlocks, async (codeBlock) => {
      const renames = await codeToVariableRenames(codeBlock);
      variablesAndFunctionsToRename =
        variablesAndFunctionsToRename.concat(renames);
    });
    return renameVariablesAndFunctions(code, variablesAndFunctionsToRename);
  };

  async function codeToVariableRenames(code: string) {
    const chatCompletion = await openai.chat.completions.create({
      model:"gpt-4o-mini",
      tools: [
        {
          "type": "function",
          "function": {
            "name": "rename_variables_and_functions",
            "description": "Rename variables and function names in Javascript code",
            "parameters": {
              "type": "object",
              "properties": {
                "variablesAndFunctionsToRename": {
                  "type": "array",
                  "items": {
                    "type": "object",
                    "properties": {
                      "name": {
                        "type": "string",
                        "description": "The name of the variable or function name to rename",
                      },
                      "newName": {
                        "type": "string",
                        "description":
                          "The new name of the variable or function name",
                      },
                    },
                    "required": ["name", "newName"],
                  },
                },
              },
              "required": ["variablesToRename"],
            },
          }
        },
      ],
      tool_choice: "auto",
      messages: [
        {
          role: "assistant",
          content:
            "Rename all Javascript variables and functions to have descriptive names based on their usage in the code.",
        },
        { role: "user", content: code },
      ],
    });
    const data = chatCompletion.choices[0];
    console.log("WE GOT HERE")
    console.log(data.finish_reason)
    if (data.finish_reason !== "tool_calls") return [];
    console.log("WE GOT HERE")
    const variablesAndFunctionsToRename = chatCompletion.choices[0].message.tool_calls.flatMap(function (value) {
      const { variablesAndFunctionsToRename }: { variablesAndFunctionsToRename: Rename[] } = JSON.parse(
        fixPerhapsBrokenResponse(value.function?.arguments!)
      );
      console.log(variablesAndFunctionsToRename)
    return variablesAndFunctionsToRename;
    });
    return variablesAndFunctionsToRename;
  };
}

function fixPerhapsBrokenResponse(jsonResponse: string) {
  // Sometimes the response has an extra comma at the end of the array, like:
  // {"result": [{"foo": "bar"}, { "foo": "baz" },\n ]}
  // This is invalid JSON, so we need to remove the comma.

  return jsonResponse.replace(/},\s*]/im, "}]");
}

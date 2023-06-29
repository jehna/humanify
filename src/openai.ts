import { Configuration, OpenAIApi } from "openai";
import { OPENAI_TOKEN } from "./env.js";
import { transform } from "@babel/core";
import { transformWithPlugins } from "./babel-utils.js";

const client = new OpenAIApi(new Configuration({ apiKey: OPENAI_TOKEN }));

export default async (code: string): Promise<string> => {
  const chatCompletion = await client.createChatCompletion({
    model: "gpt-3.5-turbo",
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
        content: "Rename Javascript variables and functions",
      },
      { role: "user", content: code },
    ],
  });
  const data = chatCompletion.data.choices[0];
  if (data.finish_reason !== "function_call") return code;

  const {
    variablesAndFunctionsToRename,
  }: { variablesAndFunctionsToRename: { name: string; newName: string }[] } =
    JSON.parse(data.message?.function_call?.arguments!);

  return renameVariablesAndFunctions(code, variablesAndFunctionsToRename);
};

async function renameVariablesAndFunctions(
  code: string,
  toRename: { name: string; newName: string }[]
): Promise<string> {
  return await transformWithPlugins(code, [
    {
      visitor: {
        Identifier: (path) => {
          const rename = toRename.find((r) => r.name === path.node.name);
          if (rename) path.node.name = rename.newName;
        },
      },
    },
  ]);
}

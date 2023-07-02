import { transformWithPlugins } from "../babel-utils.js";
import { isReservedWord } from "./is-reserved-word.js";

export type Rename = {
  name: string;
  newName: string;
};

export async function renameVariablesAndFunctions(
  code: string,
  toRename: Rename[]
): Promise<string> {
  return await transformWithPlugins(code, [
    {
      visitor: {
        Identifier: (path) => {
          const rename = toRename.find((r) => r.name === path.node.name);
          if (rename)
            path.node.name = isReservedWord(rename.newName)
              ? `${rename.newName}$`
              : rename.newName;
        },
      },
    },
  ]);
}

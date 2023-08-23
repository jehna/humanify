import { transformWithPlugins } from "./babel-utils.js";
import { createServer, send } from "./mq.js";
import { isReservedWord } from "./openai/is-reserved-word.js";

const PADDING_CHARS = 200;

export const localReanme = () => {
  createServer();

  return async (code: string): Promise<string> => {
    const { description, filename } = await send<{
      description: string;
      filename: string;
    }>({
      type: "define",
      code: code.slice(0, PADDING_CHARS * 2),
    });
    console.log(description, filename);

    let didChange = false;
    let newCode = code;
    let pos = 0;
    let renames: string[] = [];
    do {
      didChange = false;
      newCode = await transformWithPlugins(newCode, [
        {
          visitor: {
            Identifier(path) {
              if (didChange) return;
              if (path.node.name.length >= 3) return;
              const start = path.node.start ?? newCode.length;
              if (start <= pos) return;

              const { renamed } = send<{ renamed: string }>({
                type: "rename",
                before: newCode.slice(
                  Math.max(start - PADDING_CHARS, 0),
                  start
                ),
                after: newCode.slice(start, start + PADDING_CHARS),
                varname: path.node.name,
                description,
                filename,
              });
              console.log(renamed);
              let safeRenamed = isReservedWord(renamed)
                ? `_${renamed}`
                : renamed;
              while (renames.includes(safeRenamed)) {
                safeRenamed = `_${safeRenamed}`;
              }
              renames.push(safeRenamed);
              path.scope.rename(path.node.name, safeRenamed);
              didChange = true;
              pos = start;
            },
          },
        },
      ]);
      console.log(Math.round((pos / newCode.length) * 1000) / 10 + "%");
    } while (didChange);
    return newCode;
  };
};

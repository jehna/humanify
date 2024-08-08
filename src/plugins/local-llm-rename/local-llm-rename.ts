import { isValidIdentifier } from "@babel/types";
import { defineFilename } from "./define-filename.js";
import { Prompt } from "./llama.js";
import { unminifyVariableName } from "./unminify-variable-name.js";
import { visitAllIdentifiers } from "./visit-all-identifiers.js";

const PADDING_CHARS = 200;

export const localReanme = (prompt: Prompt) => {
  return async (code: string): Promise<string> => {
    const filename = await defineFilename(
      prompt,
      code.slice(0, PADDING_CHARS * 2)
    );

    const renames: string[] = [];
    return await visitAllIdentifiers(code, async (name, surroundingCode) => {
      const renamed = await unminifyVariableName(
        prompt,
        name,
        filename,
        surroundingCode
      );

      let safeRenamed = isValidIdentifier(renamed) ? `_${renamed}` : renamed;
      while (renames.includes(safeRenamed)) {
        safeRenamed = `_${safeRenamed}`;
      }
      renames.push(safeRenamed);
      return safeRenamed;
    });
  };
};

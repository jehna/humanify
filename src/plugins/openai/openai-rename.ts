import OpenAI from "openai";
import { visitAllIdentifiers } from "../local-llm-rename/visit-all-identifiers.js";
import { showPercentage } from "../../progress.js";
import { verbose } from "../../verbose.js";
import Instructor from "@instructor-ai/instructor";
import { z } from 'zod';

export function openaiRename({
  apiKey,
  baseURL,
  model,
  contextWindowSize
}: {
  apiKey: string;
  baseURL: string;
  model: string;
  contextWindowSize: number;
}) {
  const oai = new OpenAI({ apiKey, baseURL });
  const instructor = Instructor({
    client: oai,
    mode: "JSON",
  })
  return async (code: string): Promise<string> => {
    return await visitAllIdentifiers(
      code,
      async (name, surroundingCode) => {
        verbose.log(`Renaming ${name}`);
        verbose.log("Context: ", surroundingCode);

        const result = await instructor.chat.completions.create(
          toRenamePrompt(name, surroundingCode, model)
        );

        if (!result) {
          throw new Error("Failed to rename", { cause: result });
        }
        const renamed = result.newName;

        verbose.log(`Renamed to ${renamed}`);

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
  model: string
) {
  const schema = z.object({
    newName: z.string({
      description: `The new name for the variable/function called \`${name}\``
    })
  })
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
    response_model: {
      name: "Result",
      schema: schema
    }
  };
}

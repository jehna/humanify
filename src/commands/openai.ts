import { cli } from "../cli.js";
import prettier from "../plugins/prettier.js";
import { unminify } from "../unminify.js";
import babel from "../plugins/babel/babel.js";
import { openaiRename } from "../plugins/openai/openai-rename.js";

export const openai = cli()
  .name("openai")
  .description("Use OpenAI's API to unminify code")
  .option("-m, --model <model>", "The model to use", "gpt-4o-mini")
  .option("-o, --outputDir <output>", "The output directory", "output")
  .option("-k, --apiKey <apiKey>", "The OpenAI API key")
  .argument("input", "The input minified Javascript file")
  .action(async (filename, opts) => {
    const apiKey = opts.apiKey ?? process.env.OPENAI_API_KEY;
    await unminify(filename, opts.outputDir, [
      babel,
      openaiRename({ apiKey, model: opts.model }),
      prettier
    ]);
  });

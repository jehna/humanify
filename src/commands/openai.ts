import { cli } from "../cli.js";
import prettier from "../plugins/prettier.js";
import { unminify } from "../unminify.js";
import babel from "../plugins/babel/babel.js";

export const openai = cli()
  .name("openai")
  .description("Use OpenAI's API to unminify code")
  .option("-m, --model <model>", "The model to use", "gpt-4o")
  .option("-o, --outputDir <output>", "The output directory", "output")
  .argument("input", "The input minified Javascript file")
  .action(async (filename, opts) => {
    await unminify(filename, opts.outputDir, [babel, prettier]);
  });

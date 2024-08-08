import { cli } from "../cli.js";
import { llama } from "../plugins/local-llm-rename/llama.js";
import { DEFAULT_MODEL, getEnsuredModelPath } from "../local-models.js";
import { unminify } from "../unminify.js";
import prettier from "../plugins/prettier.js";
import babel from "../plugins/babel/babel.js";
import { localReanme } from "../plugins/local-llm-rename/local-llm-rename.js";

export const local = cli()
  .name("local")
  .description("Use a local LLM to unminify code")
  .showHelpAfterError(true)
  .option("-m, --model <model>", "The model to use", DEFAULT_MODEL)
  .option("-o, --outputDir <output>", "The output directory", "output")
  .argument("input", "The input minified Javascript file")
  .action(async (filename, opts) => {
    const prompt = await llama({ modelPath: getEnsuredModelPath(opts.model) });
    await unminify(filename, opts.outputDir, [
      babel,
      localReanme(prompt),
      prettier
    ]);
  });

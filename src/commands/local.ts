import { existsSync } from "fs";
import { cli } from "../cli.js";
import { llama } from "../llama.js";
import { DEFAULT_MODEL, getEnsuredModelPath } from "../local-models.js";
import { err } from "../cli-error.js";

export const local = cli()
  .name("local")
  .description("Use a local LLM to unminify code")
  .showHelpAfterError(true)
  .option("-m, --model <model>", "The model to use", DEFAULT_MODEL)
  .option("-o, --outputDir <output>", "The output directory", "output")
  .argument("input", "The input minified Javascript file")
  .action(async (filename, opts) => {
    if (!existsSync(filename)) {
      err(`File ${filename} not found`);
    }
    const model = await llama({ modelPath: getEnsuredModelPath(opts.model) });
    console.log(filename, opts);
    console.log(model);
  });

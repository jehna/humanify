import { cli } from "../cli.js";
import { unminify } from "../unminify.js";
import prettier from "../plugins/prettier.js";
import babel from "../plugins/babel/babel.js";
import { verbose } from "../verbose.js";
import { ollamaRename } from "../plugins/ollama-rename.js";

export const ollama = cli()
  .name("ollama")
  .description("Use Ollama LLM to unminify code")
  .showHelpAfterError(true)
  .option("-m, --model <model>", "The model to use (check ollama website for list of models)", 'phi3:mini')
  .option("-o, --outputDir <output>", "The output directory", "output")
  .option(
    "-s, --seed <seed>",
    "Seed for the model to get reproduceable results (leave out for random seed)"
  )
  .option("--disableGpu", "Disable GPU acceleration")
  .option("--verbose", "Show verbose output")
  .argument("input", "The input minified Javascript file")
  .action(async (filename, opts) => {
    if (opts.verbose) {
      verbose.enabled = true;
    }
    await unminify(filename, opts.outputDir, [
      babel,
      ollamaRename({model: opts.model}),
      prettier
    ]);
  });

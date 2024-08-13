import { cli } from "../cli.js";
import prettier from "../plugins/prettier.js";
import { unminify } from "../unminify.js";
import babel from "../plugins/babel/babel.js";
import { verbose } from "../verbose.js";
import { env } from "../env.js";
import { azureRename } from "../plugins/azure-rename.js";

export const azure = cli()
  .name("azure")
  .description("Use Github/Azure API to unminify code")
  .option("-m, --model <model>", "The model to use", "meta-llama-3-8b-instruct")
  .option("-o, --outputDir <output>", "The output directory", "output")
  .option(
    "-k, --apiKey <apiKey>",
    "The Github/Azure API key. Alternatively use GITHUB_TOKEN environment variable"
  )
  .option("--verbose", "Show verbose output")
  .argument("input", "The input minified Javascript file")
  .action(async (filename, opts) => {
    if (opts.verbose) {
      verbose.enabled = true;
    }

    const apiKey = opts.apiKey ?? env("GITHUB_TOKEN");
    await unminify(filename, opts.outputDir, [
      babel,
      azureRename({ apiKey, model: opts.model }),
      prettier
    ]);
  });

import { cli } from "../cli.js";
import prettier from "../plugins/prettier.js";
import { unminify } from "../unminify.js";
import babel from "../plugins/babel/babel.js";
import { ollamaRename } from "../plugins/ollama/ollama-rename.js";
import { verbose } from "../verbose.js";
import { env } from "../env.js";
import { parseNumber } from "../number-utils.js";
import { DEFAULT_CONTEXT_WINDOW_SIZE } from "./default-args.js";

export const ollama = cli()
  .name("ollama")
  .description("Use Ollama to unminify code")
  .option("-m, --model <model>", "The model to use", "gpt-oss:20b")
  .option("-o, --outputDir <output>", "The output directory", "output")
  .option(
    "--baseURL <baseURL>",
    "The Ollama base server URL.",
    env("OLLAMA_BASE_URL") ?? "http://localhost:11434"
  )
  .option("--verbose", "Show verbose output")
  .option(
    "--contextSize <contextSize>",
    "The context size to use for the LLM",
    `${DEFAULT_CONTEXT_WINDOW_SIZE}`
  )
  .argument("input", "The input minified Javascript file")
  .action(async (filename, opts) => {
    if (opts.verbose) {
      verbose.enabled = true;
    }

    const baseURL = opts.baseURL;
    const contextWindowSize = parseNumber(opts.contextSize);
    await unminify(filename, opts.outputDir, [
      babel,
      ollamaRename({
        baseURL,
        model: opts.model,
        contextWindowSize
      }),
      prettier
    ]);
  });

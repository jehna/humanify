import { cli } from "../cli.js";
import prettier from "../plugins/prettier.js";
import { unminify } from "../unminify.js";
import babel from "../plugins/babel/babel.js";
import { verbose } from "../verbose.js";
import { anthropicRename } from "../plugins/anthropic-rename.js";
import { env } from "../env.js";
import { parseNumber } from "../number-utils.js";
import { DEFAULT_CONTEXT_WINDOW_SIZE } from "./default-args.js";

export const anthropic = cli()
  .name("anthropic")
  .description("Use Anthropic's Claude API to unminify code")
  .option("-m, --model <model>", "The model to use", "claude-3-sonnet-20240229")
  .option("-o, --outputDir <output>", "The output directory", "output")
  .option(
    "-k, --apiKey <apiKey>",
    "The Anthropic API key. Alternatively use ANTHROPIC_API_KEY environment variable"
  )
  .option(
    "--baseURL <baseURL>",
    "The Anthropic base server URL.",
    env("ANTHROPIC_BASE_URL") ?? "https://api.anthropic.com"
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
    const apiKey = opts.apiKey ?? env("ANTHROPIC_API_KEY");
    const baseURL = opts.baseURL;
    const contextWindowSize = parseNumber(opts.contextSize);

    await unminify(filename, opts.outputDir, [
      babel,
      anthropicRename({
        apiKey,
        baseURL,
        model: opts.model,
        contextWindowSize
      }),
      prettier
    ]);
  });
import { existsSync } from "fs";
import { cli } from "../cli.js";
import { err } from "../cli-error.js";

export const openai = cli()
  .name("openai")
  .description("Use OpenAI's API to unminify code")
  .option("-m, --model <model>", "The model to use", "gpt-4o")
  .option("-o, --outputDir <output>", "The output directory", "output")
  .argument("input", "The input minified Javascript file")
  .action((filename, opts) => {
    if (!existsSync(filename)) {
      err(`File ${filename} not found`);
    }
    console.log(filename, opts);
  });
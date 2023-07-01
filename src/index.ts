import fs from "fs/promises";
import prettier from "./prettier.js";
import phonetize from "./phonetize.js";
import openai from "./openai.js";
import humanify from "./humanify.js";
import yargs from "yargs/yargs";
import { ensureFileExists } from "./fs-utils.js";
import { env } from "./env.js";
import { nop } from "./plugin-utils.js";

const argv = yargs(process.argv.slice(2))
  .example("npm start example.js", "Format example.js and print to stdout")
  .example(
    "npm start -o example-formatted.js example.js",
    "Format example.js and save to example-formatted.js"
  )
  .scriptName("npm start --")
  .command("<file>", "File to format")
  .options({
    output: { type: "string", alias: "o", description: "Output file" },
    key: {
      type: "string",
      alias: "openai-key",
      description: "OpenAI key (defaults to OPENAI_TOKEN environment variable)",
    },
    local: {
      type: "boolean",
      alias: "no-openai",
      default: false,
      description: "Don't use OpenAI API, only local plugins",
    },
    "4k": {
      type: "boolean",
      alias: "use-cheaper-model",
      default: false,
      description:
        "Use the cheaper GPT-3.5 model with 4k context window (default is 16k)",
    },
  })
  .demandCommand(1)
  .help()
  .parseSync();

const filename = argv._[0] as string;

await ensureFileExists(filename);

const code = await fs.readFile(filename, "utf-8");

const PLUGINS = [
  phonetize,
  humanify,
  argv.local
    ? nop
    : openai({ apiKey: argv.key ?? env("OPENAI_TOKEN"), use4k: argv["4k"] }),
  prettier,
];

const formattedCode = await PLUGINS.reduce(
  (p, next) => p.then(next),
  Promise.resolve(code)
);

if (argv.output) {
  await fs.writeFile(argv.output, formattedCode);
} else {
  console.log(formattedCode);
}

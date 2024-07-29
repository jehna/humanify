import fs from "fs/promises";
import prettier from "./prettier.js";
import openai from "./openai/openai.js";
import humanify from "./humanify.js";
import yargs from "yargs/yargs";
import { ensureFileExists } from "./fs-utils.js";
import { env } from "./env.js";
import { nop } from "./plugin-utils.js";
import fixShadowing from "./fix-shadowing.js";
import { localReanme } from "./local-rename.js";
import { webcrack } from "./webcrack.js";

const argv = yargs(process.argv.slice(2))
  .example(
    "npm start -o example-formatted.js example.js",
    "Format example.js and save to example-formatted.js"
  )
  .scriptName("npm start --")
  .command("<file>", "File to format")
  .options({
    output: {
      type: "string",
      alias: "o",
      description: "Output file",
      require: true,
    },
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
  })
  .demandCommand(1)
  .help()
  .parseSync();

const filename = argv._[0] as string;

await ensureFileExists(filename);

const bundledCode = await fs.readFile(filename, "utf-8");

const PLUGINS = [
  humanify,
  argv.local
    ? localReanme()
    : openai({ apiKey: argv.key ?? env("OPENAI_TOKEN") }),
  prettier,
];

const extractedFiles = await webcrack(bundledCode, argv.output);

for (const file of extractedFiles) {
  const code = await fs.readFile(file.path, "utf-8");
  const formattedCode = await PLUGINS.reduce(
    (p, next) => p.then(next),
    Promise.resolve(code)
  );

  await fs.writeFile(file.path, formattedCode);
}

process.exit(0); // Kills the zeromq socket

import fs from "fs/promises";
import prettier from "./prettier.js";
import phonetize from "./phonetize.js";
import openai from "./openai.js";
import humanify from "./humanify.js";
import yargs from "yargs/yargs";
import { ensureFileExists } from "./fs-utils.js";
import { env } from "./env.js";

const argv = yargs(process.argv.slice(2))
  .example("npm start example.js", "Format example.js and print to stdout")
  .example(
    "npm start -o example-formatted.js example.js",
    "Format example.js and save to example-formatted.js"
  )
  .scriptName("npm start --")
  .command("<file>", "File to format")
  .options({
    o: { type: "string", alias: "output" },
    key: { type: "string", alias: "openai-key" },
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
  openai(argv.key ?? env("OPENAI_TOKEN")),
  prettier,
];

const formattedCode = await PLUGINS.reduce(
  (p, next) => p.then(next),
  Promise.resolve<string>(code)
);

if (argv.o) {
  await fs.writeFile(argv.o, formattedCode);
} else {
  console.log(formattedCode);
}

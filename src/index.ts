import fs from "fs/promises";
import prettier from "./prettier.js";
import phonetize from "./phonetize.js";
import { not } from "./not.js";
import openai from "./openai.js";
import humanify from "./humanify.js";

const code = await fs.readFile("./example-minified.js", "utf-8");

const PLUGINS = [not(phonetize), not(openai), humanify, prettier];

const formattedCode = await PLUGINS.reduce(
  (p, next) => p.then(next),
  Promise.resolve<string>(code)
);

await fs.writeFile("./example-formatted.js", formattedCode);

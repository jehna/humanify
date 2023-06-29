import fs from "fs/promises";
import prettier from "./prettier.js";
import babel from "./babel.js";
import { not } from "./not.js";
import openai from "./openai.js";

const code = await fs.readFile("./example-minified.js", "utf-8");

const PLUGINS = [not(babel), openai, prettier];

const formattedCode = await PLUGINS.reduce(
  (p, next) => p.then(next),
  Promise.resolve<string>(code)
);

await fs.writeFile("./example-formatted.js", formattedCode);

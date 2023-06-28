import fs from "fs/promises";
import prettier from "./prettier.js";
import babel from "./babel.js";

const code = await fs.readFile("./example-minified.js", "utf-8");

const PLUGINS = [babel, prettier];

const formattedCode = await PLUGINS.reduce(
  (p, next) => p.then(next),
  Promise.resolve<string>(code)
);

await fs.writeFile("./example-formatted.js", formattedCode);

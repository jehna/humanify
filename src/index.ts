import prettier from "prettier";
import fs from "fs/promises";

const code = await fs.readFile("./example-minified.js", "utf-8");

const formattedCode = prettier.format(code, { parser: "babel" });

await fs.writeFile("./example-formatted.js", formattedCode);

import fs from "fs/promises";
import { ensureFileExists } from "./file-utils.js";
import { webcrack } from "./plugins/webcrack.js";

export async function unminify(
  filename: string,
  outputDir: string,
  plugins: ((code: string) => Promise<string>)[] = []
) {
  ensureFileExists(filename);
  const bundledCode = await fs.readFile(filename, "utf-8");
  const extractedFiles = await webcrack(bundledCode, outputDir);

  for (const file of extractedFiles) {
    const code = await fs.readFile(file.path, "utf-8");
    const formattedCode = await plugins.reduce(
      (p, next) => p.then(next),
      Promise.resolve(code)
    );

    await fs.writeFile(file.path, formattedCode);
  }
}

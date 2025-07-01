import { webcrack as wc } from "webcrack";
import fs from "fs/promises";
import path from "path";

type File = {
  path: string;
};

/**
 * Deobfuscate code using WebCrack and save the extracted result(s) into
 * {@link outputDir}. Only files that are *created during this invocation* are
 * returned.  Any previously–existing files in {@link outputDir} are ignored so
 * that they are not processed a second time.
 *
 * The file that WebCrack calls `deobfuscated.js` is renamed to
 * `[TIMESTAMP]_[ORIGINAL_FILENAME]_deobfuscated.js` so multiple runs can safely
 * coexist in the same output directory without clashes.
 */
export async function webcrack(
  code: string,
  outputDir: string,
  originalFilename: string
): Promise<File[]> {
  const startTime = Date.now();

  const cracked = await wc(code);
  await cracked.save(outputDir);

  const dirEntries = await fs.readdir(outputDir);
  const timestamp = new Date().toISOString().replace(/[:.]/g, "-");

  const result: File[] = [];

  for (const entry of dirEntries) {
    if (!entry.endsWith(".js")) continue;

    const fullPath = path.join(outputDir, entry);
    const stats = await fs.stat(fullPath);

    // Skip files that pre-date this run
    if (stats.mtimeMs < startTime) continue;

    let finalName = entry;

    // Rename generic output to a timestamped, descriptive name
    if (entry === "deobfuscated.js") {
      const base = path.basename(originalFilename, path.extname(originalFilename));
      finalName = `${timestamp}_${base}_deobfuscated.js`;
      await fs.rename(fullPath, path.join(outputDir, finalName));
    }

    result.push({ path: path.join(outputDir, finalName) });
  }

  return result;
}

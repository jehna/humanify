import fs from "fs/promises";
import { resolve } from "path";

export const ensureFileExists = async (path: string): Promise<void> => {
  try {
    await fs.access(path);
  } catch (e) {
    const fullPath = resolve(path);
    console.error(`File ${fullPath} does not exist`);
    process.exit(1);
  }
};

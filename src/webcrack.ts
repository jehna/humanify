import { webcrack as wc } from "webcrack";
import fs from "fs/promises";
import path from "path";

type File = {
  path: string;
};

export async function webcrack(code: string, outDir: string): Promise<File[]> {
  const cracked = await wc(code);
  await cracked.save(outDir);

  const output = await fs.readdir(outDir);
  return output
    .filter((file) => file.endsWith(".js"))
    .map((file) => ({ path: path.join(outDir, file) }));
}

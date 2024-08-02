import fs from "fs/promises";
import { createWriteStream, existsSync } from "fs";
import { basename } from "path";
import { Readable } from "stream";
import { finished } from "stream/promises";
import { url } from "./url.js";
import { showProgress } from "./progress.js";
import { err } from "./cli-error.js";

const MODEL_DIRECTORY = "models";

export const MODELS: { [modelName: string]: URL } = {
  "2gb": url`https://huggingface.co/bartowski/Phi-3.1-mini-4k-instruct-GGUF/resolve/main/Phi-3.1-mini-4k-instruct-Q4_K_M.gguf?download=true`
};

async function ensureModelDirectory() {
  await fs.mkdir(MODEL_DIRECTORY, { recursive: true });
}

export async function downloadModel(model: string) {
  await ensureModelDirectory();
  const url = MODELS[model];
  if (url === undefined) {
    err(`Model ${model} not found`);
  }

  const path = getModelPath(model);

  if (existsSync(path)) {
    console.log(`Model "${model}" already downloaded`);
    return;
  }

  const response = await fetch(url);
  if (!response.ok || !response.body) {
    err(`Failed to download model ${model}`);
  }
  const tmpPath = `${path}.part`;
  const fileStream = createWriteStream(tmpPath);
  const readStream = Readable.fromWeb(response.body);

  showProgress(readStream);
  await finished(readStream.pipe(fileStream));
  await fs.rename(tmpPath, path);
  process.stdout.clearLine(0);
  console.log(`Model "${model}" downloaded to ${path}`);
}

export const DEFAULT_MODEL = Object.keys(MODELS)[0];

export function getModelPath(model: string) {
  if (!(model in MODELS)) {
    err(`Model ${model} not found`);
  }
  const filename = basename(MODELS[model].pathname);
  return `${MODEL_DIRECTORY}/${filename}`;
}

export function getEnsuredModelPath(model: string) {
  const path = getModelPath(model);
  if (!existsSync(path)) {
    err(
      `Model "${model}" not found. Run "humanify download ${model}" to download the model.`
    );
  }
  return path;
}

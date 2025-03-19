import fs from "fs/promises";
import { existsSync } from "fs";
import { basename } from "path";
import { url } from "./url.js";
import { err } from "./cli-error.js";
import { homedir } from "os";
import { join } from "path";
import { ChatWrapper, Llama3_1ChatWrapper } from "node-llama-cpp";
import { downloadFile } from "ipull";
import { verbose } from "./verbose.js";

const MODEL_DIRECTORY = join(homedir(), ".humanifyjs", "models");

type ModelDefinition = { url: URL; wrapper?: ChatWrapper };

export const MODELS: { [modelName: string]: ModelDefinition } = {
  "2b": {
    url: url`https://huggingface.co/bartowski/Phi-3.5-mini-instruct-GGUF/resolve/main/Phi-3.5-mini-instruct-Q4_K_M.gguf?download=true`
  },
  "8b": {
    url: url`https://huggingface.co/lmstudio-community/Meta-Llama-3.1-8B-Instruct-GGUF/resolve/main/Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf?download=true`,
    wrapper: new Llama3_1ChatWrapper()
  }
};

async function ensureModelDirectory() {
  await fs.mkdir(MODEL_DIRECTORY, { recursive: true });
}

export function getModelWrapper(model: string) {
  if (!(model in MODELS)) {
    err(`Model ${model} not found`);
  }
  return MODELS[model].wrapper;
}

export async function downloadModel(model: string) {
  await ensureModelDirectory();
  const url = MODELS[model].url;
  if (url === undefined) {
    err(`Model ${model} not found`);
  }

  const path = getModelPath(model);

  if (existsSync(path)) {
    console.log(`Model "${model}" already downloaded`);
    return;
  }

  const tmpPath = `${path}.part`;

  const downlaoder = await downloadFile({
    url: url.toString(),
    savePath: tmpPath,
    cliProgress: true,
    cliStyle: verbose.enabled ? "ci" : "auto"
  });
  await downlaoder.download();

  await fs.rename(tmpPath, path);
  console.log(`Model "${model}" downloaded to ${path}`);
}

export const DEFAULT_MODEL = Object.keys(MODELS)[0];

export function getModelPath(model: string) {
  if (!(model in MODELS)) {
    err(`Model ${model} not found`);
  }
  const filename = basename(MODELS[model].url.pathname);
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

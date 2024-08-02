#!/usr/bin/env node
import fs from 'fs/promises';
import { existsSync, createWriteStream } from 'fs';
import { basename } from 'path';
import { Readable } from 'stream';
import { finished } from 'stream/promises';
import { Command } from 'commander';
import { getLlama, LlamaChatSession, GemmaChatWrapper, LlamaGrammar } from 'node-llama-cpp';

var version = "2.0.0";

function url(strings, ...values) {
  return new URL(String.raw(strings, ...values));
}

function showProgress(stream) {
  let bytes = 0;
  let i = 0;
  stream.on("data", (data) => {
    if (i++ % 1e3 !== 0) return;
    bytes += data.length;
    process.stdout.clearLine(0);
    process.stdout.write(`\rDownloaded ${formatBytes(bytes)}`);
  });
}
function formatBytes(numBytes) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let unitIndex = 0;
  while (numBytes > 1024 && unitIndex < units.length) {
    numBytes /= 1024;
    unitIndex++;
  }
  return `${numBytes.toFixed(2)} ${units[unitIndex]}`;
}

function err(message, exitCode = 1) {
  console.error(`\x1B[31m${message}\x1B[0m`);
  process.exit(exitCode);
}

const MODEL_DIRECTORY = "models";
const MODELS = {
  "2gb": url`https://huggingface.co/bartowski/Phi-3.1-mini-4k-instruct-GGUF/resolve/main/Phi-3.1-mini-4k-instruct-Q4_K_M.gguf?download=true`
};
async function ensureModelDirectory() {
  await fs.mkdir(MODEL_DIRECTORY, { recursive: true });
}
async function downloadModel(model) {
  await ensureModelDirectory();
  const url2 = MODELS[model];
  if (url2 === void 0) {
    throw new Error(`Model ${model} not found`);
  }
  const path = getModelPath(model);
  if (existsSync(path)) {
    console.log(`Model "${model}" already downloaded`);
    return;
  }
  const response = await fetch(url2);
  if (!response.ok || !response.body) {
    throw new Error(`Failed to download model ${model}`);
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
const DEFAULT_MODEL = Object.keys(MODELS)[0];
function getModelPath(model) {
  if (!(model in MODELS)) {
    throw new Error(`Model ${model} not found`);
  }
  const filename = basename(MODELS[model].pathname);
  return `${MODEL_DIRECTORY}/${filename}`;
}
function getEnsuredModelPath(model) {
  const path = getModelPath(model);
  if (!existsSync(path)) {
    err(
      `Model "${model}" not found. Run "humanify download ${model}" to download the model.`
    );
  }
  return path;
}

function cli() {
  const command = new Command();
  command.showHelpAfterError(true).showSuggestionAfterError(false);
  return command;
}

function download() {
  const command = cli().name("download").description("Download supported models for local consumption");
  for (const model in MODELS) {
    command.command(model).description(`Download the ${model} model`).action(() => downloadModel(model));
  }
  return command;
}

async function llama(opts) {
  const llama2 = await getLlama();
  const model = await llama2.loadModel({
    modelPath: opts == null ? void 0 : opts.modelPath
  });
  const context = await model.createContext({ seed: opts == null ? void 0 : opts.seed });
  return async (systemPrompt, userPrompt, responseGrammar) => {
    const session = new LlamaChatSession({
      contextSequence: context.getSequence(),
      autoDisposeSequence: true,
      chatWrapper: new GemmaChatWrapper(),
      systemPrompt
    });
    const response = await session.promptWithMeta(userPrompt, {
      temperature: 0.8,
      grammar: new LlamaGrammar(llama2, {
        grammar: `${responseGrammar}`
      }),
      stopOnAbortSignal: true
    });
    session.dispose();
    return responseGrammar.parseResult(response.responseText);
  };
}

const local = cli().name("local").description("Use a local LLM to unminify code").showHelpAfterError(true).option("-m, --model <model>", "The model to use", DEFAULT_MODEL).option("-o, --outputDir <output>", "The output directory", "output").argument("input", "The input minified Javascript file").action(async (filename, opts) => {
  if (!existsSync(filename)) {
    err(`File ${filename} not found`);
  }
  const model = await llama({ modelPath: getEnsuredModelPath(opts.model) });
  console.log(filename, opts);
  console.log(model);
});

const openai = cli().name("openai").description("Use OpenAI's API to unminify code").option("-m, --model <model>", "The model to use", "gpt-4o").option("-o, --outputDir <output>", "The output directory", "output").argument("input", "The input minified Javascript file").action((filename, opts) => {
  if (!existsSync(filename)) {
    err(`File ${filename} not found`);
  }
  console.log(filename, opts);
});

cli().name("humanify").description("Unminify code using OpenAI's API or a local LLM").version(version).addCommand(local).addCommand(openai).addCommand(download()).parse(process.argv);

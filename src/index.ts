#!/usr/bin/env -S npx tsx
import { version } from "../package.json";
import { download } from "./commands/download.js";
import { local } from "./commands/local.js";
import { openai } from "./commands/openai.js";
import { cli } from "./cli.js";
import { ollama } from "./commands/ollama.js";

cli()
  .name("humanify")
  .description("Unminify code using OpenAI's API or a local LLM")
  .version(version)
  .addCommand(local)
  .addCommand(ollama)
  .addCommand(openai)
  .addCommand(download())
  .parse(process.argv);

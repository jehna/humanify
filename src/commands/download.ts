import { downloadModel, MODELS } from "../local-models.js";
import { cli } from "../cli.js";

export function download() {
  const command = cli()
    .name("download")
    .description("Download supported models for local consumption");

  for (const model in MODELS) {
    command
      .command(model)
      .description(`Download the ${model} model`)
      .action(() => downloadModel(model));
  }

  return command;
}

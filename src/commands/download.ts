import { downloadModel, MODELS } from "../local-models.js";
import { cli } from "../cli.js";
import { verbose } from "../verbose.js";

export function download() {
  const command = cli()
    .name("download")
    .description("Download supported models for local consumption");

  for (const model in MODELS) {
    command
      .command(model)
      .description(`Download the ${model} model`)
      .option("-v, --verbose", "Show verbose output")
      .action((opts) => {
        if (opts.verbose) {
          verbose.enabled = true;
        }
        downloadModel(model);
      });
  }

  return command;
}

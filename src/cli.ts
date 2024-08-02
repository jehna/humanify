import { Command } from "commander";

export function cli() {
  const command = new Command();

  // Set defaults
  command.showHelpAfterError(true).showSuggestionAfterError(false);

  return command;
}

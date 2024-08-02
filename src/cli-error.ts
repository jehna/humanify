export function err(message: string, exitCode = 1): never {
  console.error(`\x1b[31m${message}\x1b[0m`);
  process.exit(exitCode);
}

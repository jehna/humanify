import assert from "assert";
import { spawn } from "child_process";

export function assertMatches(actual: string, expected: string[]) {
  assert(
    expected.includes(actual),
    `Expected ${actual} to be one of ${JSON.stringify(expected)}`
  );
}

export async function humanify(...argv: string[]) {
  const process = spawn("./dist/index.mjs", argv);
  const stdout: string[] = [];
  const stderr: string[] = [];
  process.stdout.on("data", (data) => stdout.push(data.toString()));
  process.stderr.on("data", (data) => stderr.push(data.toString()));
  await new Promise((resolve, reject) =>
    process.on("close", () => {
      if (process.exitCode === 0) {
        resolve(undefined);
      } else {
        reject(new Error(`Process exited with code ${process.exitCode}`));
      }
    })
  );
  return { stdout: stdout.join(""), stderr: stderr.join("") };
}

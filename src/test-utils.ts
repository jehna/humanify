import assert from "assert";
import { spawn } from "child_process";
import { verbose } from "./verbose.js";

export function assertMatches(actual: string, expected: string[]) {
  assert(
    expected.some((str) => actual.toLowerCase().includes(str.toLowerCase())),
    `Expected ${actual} to be one of ${JSON.stringify(expected)}`
  );
}

export async function humanify(...argv: string[]) {
  const extraArgs = argv.includes("local") ? ["--seed", "1"] : [];
  // Try using built dist first, fall back to tsx if dist doesn't exist
  const { existsSync } = await import("fs");
  const command = existsSync("./dist/index.mjs") ? "./dist/index.mjs" : "npx";
  const args = existsSync("./dist/index.mjs") ? [...argv, ...extraArgs] : ["tsx", "src/index.ts", ...argv, ...extraArgs];

  const process = spawn(command, args, { shell: true });
  const stdout: string[] = [];
  const stderr: string[] = [];
  process.stdout.on("data", (data) => stdout.push(data.toString()));
  process.stderr.on("data", (data) => stderr.push(data.toString()));
  await new Promise((resolve, reject) =>
    process.on("close", () => {
      if (process.exitCode === 0) {
        resolve(undefined);
      } else {
        reject(
          new Error(
            `Process exited with code ${process.exitCode}, stderr: ${stderr.join("")}, stdout: ${stdout.join("")}`
          )
        );
      }
    })
  );
  verbose.log("stdout", stdout.join(""));
  verbose.log("stderr", stderr.join(""));

  return { stdout: stdout.join(""), stderr: stderr.join("") };
}

export function ensure<T>(
  value: NonNullable<T> | undefined | null,
  message: string = "Value was null or undeined"
): NonNullable<T> {
  if (value === undefined || value === null) {
    throw new Error(message);
  }
  return value;
}

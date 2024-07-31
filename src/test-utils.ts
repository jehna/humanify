import assert from "assert";

export function assertMatches(actual: string, expected: string[]) {
  assert(
    expected.includes(actual),
    `Expected ${actual} to be one of ${JSON.stringify(expected)}`
  );
}

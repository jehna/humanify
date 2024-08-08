import assert from "node:assert";
import test from "node:test";
import { humanify } from "./test-utils.js";

for (const cmd of ["openai", "local"]) {
  test(`${cmd} throws error on missing file`, async () => {
    await assert.rejects(humanify(cmd, "nonexistent-file.js"));
  });
}

test("local throws error on missing model", async () => {
  await assert.rejects(humanify("local", "--model", "nonexistent-model"));
});

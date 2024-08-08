import test from "node:test";
import { gbnf } from "./gbnf.js";
import assert from "node:assert";

test("regular string yields full string", () => {
  const parsed = gbnf`hello`;
  assert.equal(parsed, `root ::= "hello"`);
  assert.equal(parsed.parseResult("hello"), "hello");
});

test("only one variable per rule is supported", () => {
  assert.throws(() => {
    assert(gbnf`'${/[a-z]+/}' '${/[a-z]+/}'`);
  });
});

test("variable yields matched string", () => {
  const parsed = gbnf`Hello ${/[a-z]+/}!`;
  assert.equal(parsed, `root ::= "Hello " [a-z]+ "!"`);
  assert.equal(parsed.parseResult("Hello world!"), "world");
});

test("works with multiple variables if one of them is a string", () => {
  const parsed = gbnf`Hello ${"there"} ${/[a-z]+/} ${"and everyone else"}!`;
  assert.equal(
    parsed,
    `root ::= "Hello " "there" " " [a-z]+ " " "and everyone else" "!"`
  );
  assert.equal(
    parsed.parseResult("Hello there world and everyone else!"),
    "world"
  );
});

test("Escapes double quotes in strings", () => {
  const parsed = gbnf`Well "hello" ${/[a-z]+/} ${'"nice"'} to meet you!`;
  assert.equal(
    parsed,
    `root ::= "Well \\"hello\\" " [a-z]+ " " "\\"nice\\"" " to meet you!"`
  );
  assert.equal(
    parsed.parseResult(`Well "hello" world "nice" to meet you!`),
    "world"
  );
});

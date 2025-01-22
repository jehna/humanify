import assert from "assert";
import test from "node:test";
import { visitAllIdentifiers } from "./visit-all-identifiers.js";

test("no-op returns the same code", async () => {
  const code = `const a = 1;`;
  assert.equal(
    code,
    await visitAllIdentifiers(code, async (name) => name, 200)
  );
});

test("no-op returns the same empty code", async () => {
  const code = "";
  assert.equal(
    code,
    await visitAllIdentifiers(code, async (name) => name, 200)
  );
});

test("renames a simple variable", async () => {
  const code = `const a = 1;`;
  assert.equal(
    `const b = 1;`,
    await visitAllIdentifiers(code, async () => "b", 200)
  );
});

test("renames variables even if they have different scopes", async () => {
  const code = `
const a = 1;
(function () {
  a = 2;
});
  `.trim();
  const expected = `
const b = 1;
(function () {
  b = 2;
});
  `.trim();
  assert.equal(expected, await visitAllIdentifiers(code, async () => "b", 200));
});

test("renames two scopes, starting from largest scope to smallest", async () => {
  const code = `
const a = 1;
(function () {
  const b = 2;
});
  `.trim();
  const expected = `
const c = 1;
(function () {
  const d = 2;
});
  `.trim();
  let i = 0;
  const result = await visitAllIdentifiers(
    code,
    async () => ["c", "d"][i++],
    200
  );
  assert.equal(expected, result);
});

test("renames shadowed variables", async () => {
  const code = `
const a = 1;
(function () {
  const a = 2;
});
    `.trim();
  const expected = `
const c = 1;
(function () {
  const d = 2;
});
    `.trim();
  let i = 0;
  const result = await visitAllIdentifiers(
    code,
    async () => ["c", "d"][i++],
    200
  );
  assert.equal(expected, result);
});

test(`does not rename class methods`, async () => {
  const code = `
class Foo {
  bar() {}
}
    `.trim();
  const expected = `
class _Foo {
  bar() {}
}`.trim();
  assert.equal(
    await visitAllIdentifiers(code, async (name) => "_" + name, 200),
    expected
  );
});

test("passes surrounding scope as an argument", async () => {
  const code = `
const a = 1;
function foo() {
  const b = 2;

  class Bar {
    baz = 3;
    hello() {
      const y = 123;
    }
  }
};
    `.trim();

  const varnameScopeTuples: [string, string][] = [];
  await visitAllIdentifiers(
    code,
    async (name, scope) => {
      varnameScopeTuples.push([name, scope]);
      return name + "_changed";
    },
    200
  );
  assert.deepEqual(varnameScopeTuples, [
    [
      "a",
      "const a = 1;\nfunction foo() {\n  const b = 2;\n  class Bar {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n}\n;"
    ],
    [
      "foo",
      "function foo() {\n  const b = 2;\n  class Bar {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n}"
    ],
    [
      "b",
      "function foo_changed() {\n  const b = 2;\n  class Bar {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n}"
    ],
    ["Bar", "class Bar {\n  baz = 3;\n  hello() {\n    const y = 123;\n  }\n}"],
    ["y", "hello() {\n  const y = 123;\n}"]
  ]);
});

test("scopes are renamed from largest to smallest", async () => {
  const code = `
function foo() {
  function bar() {
    function baz() {
    }
  }
  function qux() {
  }
}`.trim();
  const names: string[] = [];
  await visitAllIdentifiers(
    code,
    async (name) => {
      names.push(name);
      return name;
    },
    200
  );
  assert.deepEqual(names, ["foo", "bar", "baz", "qux"]);
});

test("should rename each variable only once", async () => {
  const code = `
function a(e, t) {
  var n = [];
  var r = e.length;
  var i = 0;
  for (; i < r; i += t) {
    if (i + t < r) {
      n.push(e.substring(i, i + t));
    } else {
      n.push(e.substring(i, r));
    }
  }
  return n;
}`.trim();
  const names: string[] = [];
  await visitAllIdentifiers(
    code,
    async (name) => {
      names.push(name);
      return name + "_changed";
    },
    200
  );
  assert.deepEqual(names, ["a", "e", "t", "n", "r", "i"]);
});

test("should have a scope from where the variable was declared", async () => {
  const code = `
function foo() {
  let a = 1;
  if (a == 2) {
    if (a == 1) {
      a.toString();
    }
  }
}
  `.trim();
  let scope: string | undefined;
  await visitAllIdentifiers(
    code,
    async (name, surroundingCode) => {
      if (name === "a") {
        scope = surroundingCode;
      }
      return name;
    },
    200
  );
  assert.equal(scope, code);
});

test("should not rename object properties", async () => {
  const code = `
const c = 2;
const a = {
  b: c
};
a.b;
  `.trim();
  const expected = `
const d = 2;
const e = {
  b: d
};
e.b;
  `.trim();
  assert.equal(
    expected,
    await visitAllIdentifiers(
      code,
      async (name) => {
        if (name === "c") return "d";
        if (name === "a") return "e";
        return "_" + name;
      },
      200
    )
  );
});

test("should handle invalid identifiers", async () => {
  const code = `const a = 1`;
  const result = await visitAllIdentifiers(
    code,
    async () => "this.kLength",
    200
  );
  assert.equal(result, "const thisKLength = 1;");
});

test("should handle space in identifier name (happens for some reason though it shouldn't)", async () => {
  const code = `const a = 1`;
  const result = await visitAllIdentifiers(code, async () => "foo bar", 200);
  assert.equal(result, "const fooBar = 1;");
});

test("should handle reserved identifiers", async () => {
  const code = `const a = 1`;
  const result = await visitAllIdentifiers(code, async () => "static", 200);
  assert.equal(result, "const _static = 1;");
});

test("should handle multiple identifiers named the same", async () => {
  const code = `
const a = 1;
const b = 1;
`.trim();
  const result = await visitAllIdentifiers(code, async () => "foo", 200);
  assert.equal(
    result,
    `
const foo = 1;
const _foo = 1;
`.trim()
  );
});

test("should handle multiple properties with the same name", async () => {
  const code = `
const foo = 1;
const bar = 2;
`.trim();
  const result = await visitAllIdentifiers(code, async () => "bar", 200);
  assert.equal(
    result,
    `
const _bar = 1;
const bar = 2;
`.trim()
  );
});

test("should not craash to 'arguments' assigning", async () => {
  const code = `
function foo() {
  arguments = '??';
}
`.trim();
  const result = await visitAllIdentifiers(code, async () => "foobar", 200);
  assert.equal(
    result,
    `
function foobar() {
  arguments = '??';
}
    `.trim()
  );
});

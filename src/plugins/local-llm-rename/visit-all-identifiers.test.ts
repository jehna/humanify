import assert from "assert";
import test from "node:test";
import { visitAllIdentifiers } from "./visit-all-identifiers.js";

const BABEL_SUPPORTS_RENAMING_CLASS_METHODS =
  false; /* Babel seems to not support class method renames at the moment */

test("no-op returns the same code", async () => {
  const code = `const a = 1;`;
  assert.equal(code, await visitAllIdentifiers(code, async (name) => name));
});

test("renames a simple variable", async () => {
  const code = `const a = 1;`;
  assert.equal(
    `const b = 1;`,
    await visitAllIdentifiers(code, async () => "b")
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
  assert.equal(expected, await visitAllIdentifiers(code, async () => "b"));
});

test("renames two scopes, starting from smalles scope to largest", async () => {
  const code = `
const a = 1;
(function () {
  const b = 2;
});
  `.trim();
  const expected = `
const b = 1;
(function () {
  const c = 2;
});
  `.trim();
  let i = 0;
  const result = await visitAllIdentifiers(code, async () => ["c", "b"][i++]);
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
const b = 1;
(function () {
  const c = 2;
});
    `.trim();
  let i = 0;
  const result = await visitAllIdentifiers(code, async () => ["c", "b"][i++]);
  assert.equal(expected, result);
});

test(
  "a variable in a class method should have a context of the class method",
  { skip: !BABEL_SUPPORTS_RENAMING_CLASS_METHODS },
  async () => {
    const code = `
class Foo {
  bar() {
    const a = 1;
  }
}
    `.trim();

    let asserted = false;
    await visitAllIdentifiers(code, async (name, scope) => {
      if (name === "a") {
        assert.equal(scope, "bar() {\n  const a = 1;\n}");
        asserted = true;
      }
      return name;
    });

    assert.ok(asserted);
  }
);

test(
  `renames a class method`,
  {
    skip: !BABEL_SUPPORTS_RENAMING_CLASS_METHODS
  },
  async () => {
    const code = `
class Foo {
  bar() {}
}
    `.trim();
    const expected = `
class Foo {
  baz() {}
}`.trim();
    assert.equal(
      await visitAllIdentifiers(code, async (name) =>
        name.replace("bar", "baz")
      ),
      expected
    );
  }
);

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
  await visitAllIdentifiers(code, async (name, scope) => {
    varnameScopeTuples.push([name, scope]);
    return name + "_changed";
  });
  assert.deepEqual(varnameScopeTuples, [
    ["Bar", "class Bar {\n  baz = 3;\n  hello() {\n    const y = 123;\n  }\n}"],
    [
      "b",
      "function foo() {\n  const b = 2;\n  class Bar_changed {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n}"
    ],
    [
      "foo",
      "function foo() {\n  const b_changed = 2;\n  class Bar_changed {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n}"
    ],
    [
      "hello",
      "const a = 1;\nfunction foo_changed() {\n  const b_changed = 2;\n  class Bar_changed {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n}\n;"
    ],
    [
      "baz",
      "const a = 1;\nfunction foo_changed() {\n  const b_changed = 2;\n  class Bar_changed {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n}\n;"
    ],
    [
      "a",
      "const a = 1;\nfunction foo_changed() {\n  const b_changed = 2;\n  class Bar_changed {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n}\n;"
    ]
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
  await visitAllIdentifiers(code, async (name) => {
    names.push(name);
    return name;
  });
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
  await visitAllIdentifiers(code, async (name, scope) => {
    console.log(name, scope);
    names.push(name);
    return name + "_changed";
  });
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
  await visitAllIdentifiers(code, async (name, surroundingCode) => {
    if (name === "a") {
      scope = surroundingCode;
    }
    return name;
  });
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
    await visitAllIdentifiers(code, async (name) => {
      if (name === "c") return "d";
      if (name === "a") return "e";
      return "_" + name;
    })
  );
});

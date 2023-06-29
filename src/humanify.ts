import { PluginItem } from "@babel/core";
import * as t from "@babel/types";
import { transformWithPlugins } from "./babel-utils.js";

const getBooleansBack: PluginItem = {
  visitor: {
    // Convert !1 to false and !0 to true
    UnaryExpression(path) {
      if (
        path.node.operator === "!" &&
        path.node.argument.type === "NumericLiteral"
      ) {
        path.replaceWith({
          type: "BooleanLiteral",
          value: !path.node.argument.value,
        });
      }
    },
  },
};

const convertVoidToUndefined: PluginItem = {
  visitor: {
    // Convert void 0 to undefined
    UnaryExpression(path) {
      if (
        path.node.operator === "void" &&
        path.node.argument.type === "NumericLiteral"
      ) {
        path.replaceWith({
          type: "Identifier",
          name: "undefined",
        });
      }
    },
  },
};

const unwrapSequenceExpression: PluginItem = {
  visitor: {
    // Unwrap sequence expressions, like `return a=[],a.push(1),a.unshift(2)`
    SequenceExpression(path) {
      const node = path.node;
      if (node.expressions.length > 1) {
        for (let i = 0; i < node.expressions.length - 1; i++) {
          // Discard if the first expression is just literal "0" (for some reason this is a thing 🤷‍♂️)
          const isLast = i === node.expressions.length - 1;
          if (!isLast && t.isLiteral(node.expressions[i])) {
            continue;
          }

          path.getStatementParent()?.insertBefore({
            ...node,
            expressions: [node.expressions[i]],
          });
        }
        path.replaceWith(node.expressions[node.expressions.length - 1]);
      }
    },
  },
};

const unmangleIfElse: PluginItem = {
  visitor: {
    LogicalExpression(path) {
      // Decompress `foo && bar()` to `if (foo) bar()`
      const node = path.node;
      if (!t.isExpressionStatement(path.parent)) {
        return;
      }

      switch (node.operator) {
        case "&&":
          return path.parentPath.replaceWith(
            t.ifStatement(
              node.left,
              t.blockStatement([t.expressionStatement(node.right)])
            )
          );
        case "||":
          return path.parentPath.replaceWith(
            t.ifStatement(
              t.unaryExpression("!", node.left),
              t.blockStatement([t.expressionStatement(node.right)])
            )
          );
      }
    },
  },
};

const unmangleTernariesToIfElse: PluginItem = {
  visitor: {
    ConditionalExpression(path) {
      // Decompress `foo ? bar() : baz()` to `if (foo) { bar() } else { baz() }`
      const node = path.node;
      if (!t.isExpressionStatement(path.parent)) {
        return;
      }

      return path.parentPath.replaceWith(
        t.ifStatement(
          node.test,
          t.blockStatement([t.expressionStatement(node.consequent)]),
          t.blockStatement([t.expressionStatement(node.alternate)])
        )
      );
    },
  },
};

const convertMultiVarDeclarationToMultipleDeclarations: PluginItem = {
  visitor: {
    VariableDeclaration(path) {
      // Convert `var a=1,b=2` to `var a=1;var b=2`
      const node = path.node;
      if (node.declarations.length > 1) {
        for (let i = 0; i < node.declarations.length - 1; i++) {
          path.getStatementParent()?.insertBefore({
            ...node,
            declarations: [node.declarations[i]],
          });
        }
        path.replaceWith({
          ...node,
          declarations: [node.declarations[node.declarations.length - 1]],
        });
      }
    },
  },
};

const flipComparisonsTheRightWayAround: PluginItem = {
  visitor: {
    // If a variable is compared to a literal, flip the comparison around so that the literal is on the right-hand side
    BinaryExpression(path) {
      const node = path.node;
      const mappings: any = {
        "==": "==",
        "!=": "!=",
        "===": "===",
        "!==": "!==",
        "<": ">",
        "<=": ">=",
        ">": "<",
        ">=": "<=",
      };
      if (
        t.isLiteral(node.left) &&
        !t.isLiteral(node.right) &&
        mappings[node.operator]
      ) {
        path.replaceWith({
          ...node,
          left: node.right,
          right: node.left,
          operator: mappings[node.operator],
        });
      }
    },
  },
};

export default async (code: string): Promise<string> =>
  transformWithPlugins(code, [
    getBooleansBack,
    convertVoidToUndefined,
    unwrapSequenceExpression,
    unmangleIfElse,
    unmangleTernariesToIfElse,
    convertMultiVarDeclarationToMultipleDeclarations,
    flipComparisonsTheRightWayAround,
  ]);

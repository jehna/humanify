import { PluginItem } from "@babel/core";
import * as t from "@babel/types";
import { transformWithPlugins } from "../../babel-utils.js";
import beautifier from "babel-plugin-transform-beautifier";

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
          name: "undefined"
        });
      }
    }
  }
};

const flipComparisonsTheRightWayAround: PluginItem = {
  visitor: {
    // If a variable is compared to a literal, flip the comparison around so that the literal is on the right-hand side
    BinaryExpression(path) {
      const node = path.node;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const mappings: any = {
        "==": "==",
        "!=": "!=",
        "===": "===",
        "!==": "!==",
        "<": ">",
        "<=": ">=",
        ">": "<",
        ">=": "<="
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
          operator: mappings[node.operator]
        });
      }
    }
  }
};

const makeNumbersLonger: PluginItem = {
  visitor: {
    // Convert 5e3 to 5000
    NumericLiteral(path) {
      if (
        typeof path.node.extra?.raw === "string" &&
        path.node.extra?.raw?.includes("e")
      ) {
        path.replaceWith({
          type: "NumericLiteral",
          value: Number(path.node.extra.raw)
        });
      }
    }
  }
};

export default async (code: string): Promise<string> =>
  transformWithPlugins(code, [
    convertVoidToUndefined,
    flipComparisonsTheRightWayAround,
    makeNumbersLonger,
    bautifier
  ]);

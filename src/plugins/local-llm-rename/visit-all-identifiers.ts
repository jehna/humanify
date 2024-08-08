import { parseAsync, transformFromAstAsync, NodePath } from "@babel/core";
import babelTraverse from "@babel/traverse";
import { Identifier, Node } from "@babel/types";

const CONTEXT_WINDOW_SIZE = 200;

const traverse = babelTraverse.default; // How does one import a callble default export?

type Visitor = (name: string, scope: string) => Promise<string>;

export async function visitAllIdentifiers(code: string, visitor: Visitor) {
  const ast = await parseAsync(code);
  const visited = new Set<string>();
  if (!ast) {
    throw new Error("Failed to parse code");
  }

  while (true) {
    const smallestScope = await findIdentifierWithLargestScopeNotVisited(
      ast,
      visited
    );
    if (!smallestScope) {
      break;
    }
    const smallestScopeNode = smallestScope.node;
    if (smallestScopeNode.type !== "Identifier") {
      throw new Error("No identifiers found");
    }

    const surroundingCode = await scopeToString(smallestScope);
    const renamed = await visitor(smallestScopeNode.name, surroundingCode);
    smallestScope.scope.rename(smallestScopeNode.name, renamed);
    markVisited(smallestScope, smallestScopeNode.name, visited);
  }

  const stringified = await transformFromAstAsync(ast);
  if (!stringified?.code) {
    throw new Error("Failed to stringify code");
  }
  return stringified?.code;
}

function findIdentifierWithLargestScopeNotVisited(
  node: Node,
  visited: Set<string>
) {
  let result: NodePath<Identifier> | undefined;
  let resultSize = Infinity;

  traverse(node, {
    BindingIdentifier(path) {
      if (hasVisited(path, visited)) return;

      if (!result) {
        result = path;
        return;
      }

      const bindingBlock = closestSurroundingContextPath(path).scope.block;
      const pathSize = bindingBlock.end! - bindingBlock.start!;

      result = resultSize > pathSize ? result : path;
      resultSize = resultSize > pathSize ? resultSize : pathSize;
    }
  });

  return result;
}

function hasVisited(path: NodePath<Identifier>, visited: Set<string>) {
  return visited.has(path.node.name);
}

function markVisited(
  path: NodePath<Identifier>,
  newName: string,
  visited: Set<string>
) {
  visited.add(newName);
}

async function scopeToString(path: NodePath<Identifier>) {
  const surroundingPath = closestSurroundingContextPath(path);
  const code = `${surroundingPath}`; // Implements a hidden `.toString()`
  if (code.length < CONTEXT_WINDOW_SIZE) {
    return code;
  }
  if (surroundingPath.isProgram()) {
    const start = path.node.start ?? 0;
    const end = path.node.end ?? code.length;
    if (end < CONTEXT_WINDOW_SIZE / 2) {
      return code.slice(0, CONTEXT_WINDOW_SIZE);
    }
    if (start > code.length - CONTEXT_WINDOW_SIZE / 2) {
      return code.slice(-CONTEXT_WINDOW_SIZE);
    }

    return code.slice(
      start - CONTEXT_WINDOW_SIZE / 2,
      end + CONTEXT_WINDOW_SIZE / 2
    );
  } else {
    return code.slice(0, CONTEXT_WINDOW_SIZE);
  }
}

function closestSurroundingContextPath(
  path: NodePath<Identifier>
): NodePath<Node> {
  const programOrBindingNode = path.findParent(
    (p) => p.isProgram() || path.node.name in p.getOuterBindingIdentifiers()
  )?.scope.path;
  return programOrBindingNode ?? path.scope.path;
}

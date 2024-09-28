import { parseAsync, transformFromAstAsync, NodePath } from "@babel/core";
import * as babelTraverse from "@babel/traverse";
import { Identifier, toIdentifier, Node } from "@babel/types";

const traverse: typeof babelTraverse.default.default = (
  typeof babelTraverse.default === "function"
    ? babelTraverse.default
    : babelTraverse.default.default
) as any; // eslint-disable-line @typescript-eslint/no-explicit-any -- This hack is because pkgroll fucks up the import somehow

const CONTEXT_WINDOW_SIZE = 200;

type Visitor = (name: string, scope: string) => Promise<string>;

export async function visitAllIdentifiers(
  code: string,
  visitor: Visitor,
  onProgress?: (percentageDone: number) => void
) {
  const ast = await parseAsync(code);
  const renames = new Set<string>();
  const visited = new Set<string>();

  if (!ast) {
    throw new Error("Failed to parse code");
  }

  const scopes = await findScopes(ast);
  const numRenamesExpected = scopes.length;

  for (const smallestScope of scopes) {
    if (hasVisited(smallestScope, visited)) continue;

    const smallestScopeNode = smallestScope.node;
    if (smallestScopeNode.type !== "Identifier") {
      throw new Error("No identifiers found");
    }

    const surroundingCode = await scopeToString(smallestScope);
    const renamed = await visitor(smallestScopeNode.name, surroundingCode);

    let safeRenamed = toIdentifier(renamed);
    while (renames.has(safeRenamed)) {
      safeRenamed = `_${safeRenamed}`;
    }
    renames.add(safeRenamed);

    smallestScope.scope.rename(smallestScopeNode.name, safeRenamed);
    markVisited(smallestScope, smallestScopeNode.name, visited);

    onProgress?.(visited.size / numRenamesExpected);
  }
  onProgress?.(1);

  const stringified = await transformFromAstAsync(ast);
  if (!stringified?.code) {
    throw new Error("Failed to stringify code");
  }
  return stringified?.code;
}

function findScopes(ast: Node): NodePath<Identifier>[] {
  const scopes: [nodePath: NodePath<Identifier>, scopeSize: number][] = [];
  traverse(ast, {
    BindingIdentifier(path) {
      const bindingBlock = closestSurroundingContextPath(path).scope.block;
      const pathSize = bindingBlock.end! - bindingBlock.start!;

      scopes.push([path, pathSize]);
    }
  });

  scopes.sort((a, b) => b[1] - a[1]);

  return scopes.map(([nodePath]) => nodePath);
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

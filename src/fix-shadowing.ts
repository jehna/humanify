import { NodePath } from "@babel/core";
import { transformWithPlugins } from "./babel-utils.js";
import { Identifier } from "@babel/types";

const hasAlreadyBeenRenamed = Symbol("renamed");

export default async (code: string): Promise<string> => {
  let nameIndex = 0;
  const getNextName = () => indexToName(nameIndex++);

  const handleRename = (path: NodePath<Identifier>) => {
    const binding = path.scope.getBinding(path.node.name);
    if (!binding) {
      return;
    }
    if ((binding as any)[hasAlreadyBeenRenamed]) {
      return;
    }
    path.scope.rename(path.node.name, getNextName());
    (binding as any)[hasAlreadyBeenRenamed] = true;
  };

  return transformWithPlugins(code, [
    {
      visitor: {
        ReferencedIdentifier: handleRename,
        BindingIdentifier: handleRename,
      },
    },
  ]);
};

function indexToName(index: number): string {
  const alphabet = "abcdefghijklmnopqrstuvwxyz";
  const base = alphabet.length;
  let name = "";
  while (index > 0) {
    const remainder = index % base;
    name = alphabet[remainder] + name;
    index = Math.floor(index / base);
  }
  return name.padStart(3, "a");
}

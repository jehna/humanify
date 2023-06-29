import { transformWithPlugins } from "./babel-utils.js";

export default async (code: string): Promise<string> =>
  transformWithPlugins(code, ["transform-phonetize"]);

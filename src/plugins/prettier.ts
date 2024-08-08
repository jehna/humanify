import prettier from "prettier";

export default async (code: string): Promise<string> =>
  prettier.format(code, { parser: "babel" });

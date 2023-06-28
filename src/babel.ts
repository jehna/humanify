import { transform } from "@babel/core";

export default async (code: string): Promise<string> =>
  await new Promise((resolve, reject) =>
    transform(
      code,
      {
        plugins: ["transform-phonetize"],
        compact: false,
        minified: false,
        comments: false,
        sourceMaps: false,
        retainLines: false,
      },
      (err, result) => {
        if (err || !result) {
          reject(err);
        } else {
          resolve(result.code as string);
        }
      }
    )
  );

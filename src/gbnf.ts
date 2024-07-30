export function gbnf(
  strings: TemplateStringsArray,
  ...values: (string | RegExp)[]
) {
  return (
    "root ::= " +
    strings
      .flatMap((str, i) => [str, values[i]])
      .filter((x) => !!x)
      .map((rule) =>
        rule instanceof RegExp
          ? rule.source
          : `"${rule.replaceAll('"', '\\"')}"`
      )
      .join(" ")
  );
}

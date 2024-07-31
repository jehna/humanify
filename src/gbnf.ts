export class Gbnf {
  rule: string;
  genStart: number;
  genEnd?: number;

  constructor(rule: string, genStart: number, genEnd?: number) {
    this.rule = rule;
    this.genStart = genStart;
    this.genEnd = genEnd;
  }

  toString() {
    return this.rule;
  }

  parseResult(result: string) {
    return result.slice(this.genStart, this.genEnd);
  }
}

export function gbnf(
  strings: TemplateStringsArray,
  ...values: (string | RegExp)[]
) {
  const numRegexes = values.filter((value) => value instanceof RegExp).length;
  if (numRegexes > 1) {
    throw new Error("Only one variable per rule is supported");
  }

  let rule = "root ::=";
  for (let i = 0; i < strings.length; i++) {
    rule += ` "${strings[i].replaceAll('"', '\\"')}"`;

    const value = values[i];
    if (value instanceof RegExp) {
      rule += ` ` + value.source;
    } else if (typeof value == "string") {
      rule += ` "${value.replaceAll('"', '\\"')}"`;
    } else {
      // Undefined
    }
  }

  if (numRegexes === 0) {
    return new Gbnf(rule, 0, undefined);
  }

  let startVar = 0;
  let endVar = 0;
  let isPastRegex = false;
  for (let i = 0; i < strings.length; i++) {
    if (isPastRegex) {
      endVar -= strings[i].length;
    } else {
      startVar += strings[i].length;
    }

    const value = values[i];
    if (value instanceof RegExp) {
      isPastRegex = true;
    } else if (typeof value == "string") {
      if (isPastRegex) {
        endVar -= value.length;
      } else {
        startVar += value.length;
      }
    } else {
      // Undefined
    }
  }

  return new Gbnf(rule, startVar, endVar);
}

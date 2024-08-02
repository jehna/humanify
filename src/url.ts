export function url(strings: TemplateStringsArray, ...values: unknown[]): URL {
  return new URL(String.raw(strings, ...values));
}

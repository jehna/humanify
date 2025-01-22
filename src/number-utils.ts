export function parseNumber(value: string): number {
  const parsed = parseInt(value);
  if (isNaN(parsed)) {
    throw new Error(`Invalid number: ${value}`);
  }
  return parsed;
}

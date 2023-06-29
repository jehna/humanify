export const not: <T>(fn: (arg: T) => Promise<T>) => (arg: T) => Promise<T> =
  (_fn) => (arg) =>
    Promise.resolve(arg);

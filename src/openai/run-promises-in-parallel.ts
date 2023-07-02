export async function mapPromisesParallel<T, U>(
  numParallel: number,
  items: T[],
  fn: (item: T, index: number) => Promise<U>
): Promise<U[]> {
  const results: U[] = [];
  const promises: Promise<void>[] = [];
  let index = 0;
  while (index < items.length) {
    while (promises.length < numParallel && index < items.length) {
      promises.push(
        fn(items[index], index++).then((result) => {
          results.push(result);
        })
      );
    }
    await Promise.all(promises);
    promises.length = 0;
  }
  return results;
}

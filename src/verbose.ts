export const verbose = {
  log(...args: ConsoleLogArgs) {
    if (this.enabled) {
      const timestamp = new Date()
        .toISOString()
        .replace(/T/, " ")
        .replace(/\..+/, "");
      console.log(`[${timestamp}] `, ...args);
    }
  },
  enabled: false
};

type ConsoleLogArgs = Parameters<typeof console.log>;

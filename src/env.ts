import dotenv from "dotenv";

dotenv.config();

export const OPENAI_TOKEN = env("OPENAI_TOKEN");

function env(name: string, fallback?: string): string {
  const value = process.env[name];
  if (value === undefined) {
    if (fallback !== undefined) {
      return fallback;
    }
    throw new Error(`Missing environment variable: ${name}`);
  }
  return value;
}

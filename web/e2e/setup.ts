// Starts the throwaway Slop for the browser tests and stops it (dropping its database) after.
import { spawn } from "node:child_process";
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
export const STATE = path.join(here, ".state.json");

export default async function setup() {
  execFileSync("cargo", ["build", "-q", "-p", "slop-api", "--example", "e2e_server"], { cwd: root, stdio: "inherit" });
  const server = spawn(path.join(root, "target/debug/examples/e2e_server"), {
    cwd: root,
    env: { ...process.env, DATABASE_URL: process.env.SLOP_TEST_DATABASE_URL ?? "postgres://slop:slop@127.0.0.1:5433/slop" },
    stdio: ["pipe", "pipe", "inherit"],
  });
  const line = await new Promise<string>((resolve, reject) => {
    server.once("exit", (code) => reject(new Error(`the test server stopped (${code})`)));
    createInterface({ input: server.stdout! }).once("line", resolve);
  });
  writeFileSync(STATE, line);
  // Closing its stdin stops it and drops the database.
  return async () => {
    server.stdin!.end();
    await new Promise((r) => server.once("exit", r));
  };
}

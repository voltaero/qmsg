import { existsSync } from "node:fs";
import { chromium } from "playwright";

/** Installs only the pinned browser, never changes system libraries. */
export async function ensureBrowser() {
  if (existsSync(chromium.executablePath())) return;
  const result = Bun.spawn([
    process.execPath, "x", "--no-install", "playwright", "install", "chromium",
  ], { stdout: "ignore", stderr: "inherit", timeout: 120_000 });
  if (await result.exited !== 0) {
    throw new Error("Could not install Chromium. Check network access and run bunx playwright install chromium.");
  }
}

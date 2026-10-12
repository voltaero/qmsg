import { existsSync } from "node:fs";

// The host can start a fresh checkout without a separate npm/Bun setup step.
if (!existsSync(new URL("../node_modules/playwright/package.json", import.meta.url))) {
  const install = Bun.spawn([process.execPath, "install", "--frozen-lockfile"], {
    cwd: new URL("..", import.meta.url).pathname,
    stdout: "ignore", stderr: "inherit", timeout: 120_000,
  });
  if (await install.exited !== 0) throw new Error("Browser service dependency setup failed");
}
await import("./main");

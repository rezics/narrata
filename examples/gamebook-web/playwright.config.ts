import { defineConfig } from "@playwright/test";
import { tmpdir } from "node:os";
import { join } from "node:path";

export default defineConfig({
  testDir: "./tests",
  outputDir: join(tmpdir(), "narrata-gamebook-playwright"),
  timeout: 40_000,
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  use: { baseURL: "http://127.0.0.1:4173", browserName: "chromium", headless: true, viewport: { width: 1536, height: 1024 }, screenshot: "only-on-failure", trace: "retain-on-failure" },
  webServer: { command: "npm run dev", url: "http://127.0.0.1:4173", reuseExistingServer: !process.env.CI, timeout: 30_000 },
});

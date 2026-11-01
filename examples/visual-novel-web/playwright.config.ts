import { resolve } from "node:path";
import { defineConfig } from "@playwright/test";

const root = resolve(import.meta.dirname, "../..");

export default defineConfig({
  testDir: "./tests",
  outputDir: resolve(root, ".temp/goal/runs/vn-player/browser-artifacts"),
  timeout: 60_000,
  expect: { timeout: 12_000 },
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:4175",
    browserName: "chromium",
    headless: true,
    viewport: { width: 1586, height: 992 },
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:4175",
    reuseExistingServer: false,
    timeout: 30_000,
  },
});

import { tmpdir } from "node:os";
import { join } from "node:path";

import { defineConfig } from "@playwright/test";

// 4173 belongs to the Gamebook reader's regressions.
const port = 4183;

export default defineConfig({
  testDir: "./browser/tests",
  outputDir: join(tmpdir(), "narrata-storage-playwright"),
  timeout: 30_000,
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  use: { baseURL: `http://127.0.0.1:${port}`, browserName: "chromium", headless: true, trace: "retain-on-failure" },
  webServer: {
    command: `vite --config browser/vite.config.ts --host 127.0.0.1 --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});

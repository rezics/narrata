import { defineConfig } from "vitest/config";

export default defineConfig({ test: {
  include: ["test/server*.test.ts"],
  // PGlite has one connection and conformance exercises large boundary values.
  fileParallelism: false,
  testTimeout: 60_000,
  hookTimeout: 60_000,
} });

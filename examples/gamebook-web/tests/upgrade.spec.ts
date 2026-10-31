import { test } from "@playwright/test";
import { resolve } from "node:path";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { verifyUpdateScenarios } from "../scripts/reference-journey.ts";

for (const mobile of [false, true]) test(`visible story update and maintenance outcomes (${mobile ? "390px" : "desktop"})`, async ({ page }) => {
  test.setTimeout(180_000);
  const reader = readFileSync(resolve(import.meta.dirname, "../../../packages/narrata/web/dist/reader-wasm/narrata_nodes_wasm_bg.wasm"));
  const authoring = readFileSync(resolve(import.meta.dirname, "../../../packages/narrata/web/dist/authoring-wasm/narrata_authoring_wasm_bg.wasm"));
  const identity = (bytes: Uint8Array) => ({ bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") });
  await verifyUpdateScenarios(page, "http://127.0.0.1:4173", resolve(import.meta.dirname, `../../../.temp/update-scenarios-${mobile ? "mobile" : "desktop"}`),
    { mobile, readerWasm: identity(reader), authoringWasm: identity(authoring) });
});

import { test } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { verifyReferenceJourney } from "../scripts/reference-journey.ts";

test("cold public-authored compound reference proves identities, story mechanisms, endings and online recovery", async ({ page }) => {
  test.setTimeout(90_000);
  const wasm = readFileSync(resolve(import.meta.dirname, "../../../packages/narrata/web/dist/reader-wasm/narrata_nodes_wasm_bg.wasm"));
  await verifyReferenceJourney(page, "http://127.0.0.1:4173", resolve(import.meta.dirname, "../../../.temp/cold-reference-evidence"),
    { bytes: wasm.length, sha256: createHash("sha256").update(wasm).digest("hex") });
});

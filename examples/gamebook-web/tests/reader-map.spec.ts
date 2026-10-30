import { test } from "@playwright/test";
import { resolve } from "node:path";
// @ts-expect-error Shared installed-package browser probe uses runtime assertions.
import { verifyMapJourney } from "../../../packages/narrata/web/scripts/install/map-journey.mjs";
test("server-generated revealed map has desktop and 390px paths without reader trace", async ({ page }) => {
  await verifyMapJourney(page, "http://127.0.0.1:4173", resolve(import.meta.dirname, "../../../.temp/reader-map-evidence"));
});

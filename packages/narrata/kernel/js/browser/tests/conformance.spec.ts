import { expect, test } from "@playwright/test";
import { conformanceCases } from "../../test/support/conformance";

declare global {
  var runStorageConformance: (name: string) => Promise<void>;
}

for (const scenario of conformanceCases) {
  test(`CacheStore conformance: ${scenario.name}`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/conformance.html");
    await expect(page.locator("#status")).toHaveText("ready");
    await page.evaluate(async (name) => {
      await globalThis.runStorageConformance(name);
    }, scenario.name);
    expect(errors).toEqual([]);
  });
}

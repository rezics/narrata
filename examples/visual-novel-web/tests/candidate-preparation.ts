import { expect, type Page } from "@playwright/test";

/** Optional work can be partial; these journeys need the old-hall background ready. */
export async function waitForPreparedBackground(page: Page, minimumDecodeCount = 1) {
  await expect.poll(async () => page.evaluate(() => window.__vnTestHooks?.controller.getSnapshot().candidates?.status)).toMatch(/^(complete|partial)$/);
  await expect.poll(async () => page.evaluate(minimum => {
    const hooks = window.__vnTestHooks;
    const request = hooks?.metrics.mediaRequests.filter(item => item.key === "background/old-hall").at(-1);
    const decodes = hooks?.metrics.mediaDecodes.filter(item => item.phase === "candidate" && item.key === "background/old-hall" && item.revision === "1").length ?? 0;
    return request?.status === "available" && decodes >= minimum
      && (hooks?.controller.getSnapshot().candidates?.mediaPrepared ?? 0) > 0;
  }, minimumDecodeCount)).toBe(true);
}

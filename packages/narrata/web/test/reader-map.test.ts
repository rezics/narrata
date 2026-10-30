import { readFile } from "node:fs/promises";
import { expect, test } from "vitest";
import { loadAuthoring } from "@rezics/narrata/server";
// @ts-expect-error The shared installed-package probe is plain ESM, with runtime assertions.
import { verifyMaps } from "../scripts/install/maps.mjs";
test("public Rust-backed visits, maps and sparse relations reproduce frozen native outputs", async () => {
  expect(await verifyMaps(await loadAuthoring(), async (name: string) => new Uint8Array(await readFile(new URL(`../../../../fixtures/compat/reader-map-v1/${name}`, import.meta.url))))).toBe(true);
});

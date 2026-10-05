import "fake-indexeddb/auto";
import { IDBFactory } from "fake-indexeddb";
import { describe, it } from "vitest";
import { conformanceCases, runConformanceCase } from "./support/conformance";
import { memoryFixture } from "./support/memory-store";
import { indexedDbFixture } from "./support/store-fixture";

for (const [name, factory] of [
  ["memory reference", memoryFixture],
  ["IndexedDB", () => indexedDbFixture("conformance", new IDBFactory())],
] as const) {
  describe(`CacheStore conformance: ${name}`, () => {
    for (const test of conformanceCases) it(test.name, () => runConformanceCase(test.name, factory), 30_000);
  });
}

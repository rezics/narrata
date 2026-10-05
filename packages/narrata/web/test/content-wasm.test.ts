import { expect, test } from "vitest";
import { openBook } from "@rezics/narrata";
import { MockRezicsContent, mountainLetterData, mountainLetterKeys } from "@rezics/narrata/testing";
import { demoPack, execution, splitPack } from "./fixture.js";

test("Wasm lookahead retries missing chunks, emits only units and preserves full/lazy saves", async () => {
  const parts = splitPack(demoPack);
  const loaded: string[] = [];
  const lazy = await openBook({ manifest: parts.manifest, execution, fetchChunk: async id => {
    loaded.push(id);
    const bytes = parts.objects.get(id);
    if (!bytes) throw new Error("Missing fixture chunk");
    return bytes;
  } });
  const full = await openBook({ pack: demoPack, execution });
  try {
    const save = await lazy.exportSave();
    expect(loaded).toHaveLength(1);
    const expected = await full.nextContentUnits();
    expect(await lazy.nextContentUnits()).toEqual(expected);
    expect(loaded.length).toBeGreaterThan(1);
    expect(expected).toEqual([...expected].sort((a, b) => a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
    expect(expected.some(reference => reference.key === "camp.fire")).toBe(true);
    expect(expected.some(reference => reference.key === "road.rocks")).toBe(true);
    expect(expected.every(reference => Object.keys(reference).sort().join(",") === "key,provider")).toBe(true);
    expect(await lazy.exportSave()).toBe(save);
    expect(await full.exportSave()).toBe(save);
    // The testing subpath is usable by the installed package's host, with IRI keys.
    const key = mountainLetterKeys["main.station"];
    if (!key) throw new Error("Missing sample occurrence");
    const results = await new MockRezicsContent(mountainLetterData()).resolve({ context: { realization: "en" }, items: [{ content: { unit: { provider: "rezics", key } } }] });
    expect(results[0]?.status).toBe("ok");
  } finally { await lazy.close(); await full.close(); }
});

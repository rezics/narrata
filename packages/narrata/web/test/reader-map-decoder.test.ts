import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { decodeReaderMap, readReaderMapResponse } from "@rezics/narrata/reader-map";
const fixture = () => JSON.parse(readFileSync(new URL("../../../../fixtures/compat/reader-map-v1/initial.map.json", import.meta.url), "utf8"));
test("generated map decoder checks shape, versions and exact single-cursor provenance", () => {
  const value = fixture(), identity = { artifactId: value.artifact_id, cursor: value.sources[0].cursor };
  expect(decodeReaderMap(value, identity)).toBe(value);
  for (const mutate of [
    (v: ReturnType<typeof fixture>) => { v.nodes = "bad"; },
    (v: ReturnType<typeof fixture>) => { v.format_version = 0; },
    (v: ReturnType<typeof fixture>) => { v.projection_version = 2; },
    (v: ReturnType<typeof fixture>) => { v.sources.push(structuredClone(v.sources[0])); },
    (v: ReturnType<typeof fixture>) => { v.nodes[0].x = .5; },
    (v: ReturnType<typeof fixture>) => { v.nodes[0].x = Infinity; },
    (v: ReturnType<typeof fixture>) => { v.hidden_target = "forbidden"; },
  ]) { const value = fixture(); mutate(value); expect(() => decodeReaderMap(value, identity)).toThrow(); }
  expect(() => decodeReaderMap(value, { ...identity, artifactId: "artifact:" + "00".repeat(32) })).toThrow();
  expect(() => decodeReaderMap(value, { ...identity, cursor: "commit:" + "00".repeat(32) })).toThrow();
});
test("HTTP map bodies and object counts have explicit transport bounds", async () => {
  const value = fixture(), identity = { artifactId: value.artifact_id, cursor: value.sources[0].cursor };
  expect(await readReaderMapResponse(new Response(JSON.stringify(value)), identity)).toEqual(value);
  await expect(readReaderMapResponse(new Response("{}", { headers: { "content-length": "8388609" } }), identity)).rejects.toThrow("limit");
  await expect(readReaderMapResponse(new Response(new Uint8Array(8388609)), identity)).rejects.toThrow("limit");
  value.nodes = Array(100001).fill(value.nodes[0]);
  expect(() => decodeReaderMap(value, identity)).toThrow("limit");
});

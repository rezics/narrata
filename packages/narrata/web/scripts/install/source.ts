import { strict as assert } from "node:assert";
import { decodeRecord, type Authoring, type PackageSource, type ProjectManifest } from "@rezics/narrata/server";

// A consumer constructs and edits its own source using only the installed public types.
export function constructAndEdit(api: Authoring): void {
  const manifest: ProjectManifest = {
    format_version: 2, packages: { story: "story.json" },
    product: { id: "consumer", entry: { package: "story", graph: "main" }, endings: { done: {} } },
  };
  const story: PackageSource = {
    id: "consumer.story", version: "1", exports: ["main"],
    graphs: { main: { entry: "start", outcomes: ["done"], nodes: {
      start: { type_id: "narrata.passage", data: { next: "end" } },
      end: { type_id: "narrata.return", data: { outcome: "done" } },
    } } },
  };
  let next = 1;
  const minter = api.createMinter({ now: () => 1_750_000_000_000, random: bytes => { bytes.fill(next++); } });
  const drafts = api.createDrafts();
  try {
    const filled = minter.fill(manifest, [["story", story]]);
    assert.deepEqual(filled.diagnostics, []);
    for (const checked of filled.records) {
      const record = decodeRecord(checked.bytes);
      if (record.kind === "node" && record.key === "start") {
        record.payload.data = { next: "end", choice_points: [], args: { revision: { kind: "literal", value: 2 } } };
      }
      drafts.add(record);
    }
    const assembled = drafts.assemble();
    assert.deepEqual(assembled.diagnostics, []);
    assert.deepEqual(assembled.source?.packages.get("story")?.graphs.main?.nodes.start?.data, {
      next: "end", args: { revision: { kind: "literal", value: 2 } },
    });
    const preview = drafts.preview();
    try { assert.equal(preview.compiled, true); } finally { preview.free(); }
  } finally { drafts.free(); minter.free(); }
}

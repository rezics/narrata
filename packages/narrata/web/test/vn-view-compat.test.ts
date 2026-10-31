import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { decodeVnView, openVn, type VnViewV1 } from "@rezics/narrata/vn";
import { execution, vnPack } from "./fixture.js";

const corpus = new URL("../../../../fixtures/compat/vn-view-v1/", import.meta.url);
const names = ["dialogue.json", "choice.json", "scene-transition.json", "finished.json"];
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

test("frozen public VN views retain their recorded bytes and versioned read path", () => {
  const artifacts = Object.fromEntries(names.map(name => [name, {
    sha256: sha256(readFileSync(new URL(name, corpus))),
  }]));
  expect(JSON.parse(readFileSync(new URL("manifest.json", corpus), "utf8"))).toMatchObject({
    corpus_version: 1,
    format: "ADR 0024 public @rezics/narrata/vn VnViewV1 JSON",
    source_program_sha256: sha256(vnPack),
    execution,
    artifacts,
  });
  for (const name of names) {
    const view = decodeVnView(readFileSync(new URL(name, corpus), "utf8"));
    expect(view.version).toBe(1);
    expect(view.view.scene).toBeDefined();
  }
});

test("the public VN reader preserves dialogue, choice, scene-transition and ending views", async () => {
  const reader = await openVn({ pack: vnPack, execution });
  const matches = (name: string, actual: VnViewV1) => {
    expect(actual).toEqual(decodeVnView(readFileSync(new URL(name, corpus), "utf8")));
  };
  try {
    matches("dialogue.json", await reader.inspect());
    for (let index = 0; index < 3; index++) {
      const current = await reader.inspect(), interaction = current.view.interaction;
      if (interaction.kind !== "advance") throw new Error("Expected a dialogue safe point");
      await reader.advance(current.view.cursor, interaction.dialogue);
    }
    const choice = await reader.inspect(), interaction = choice.view.interaction;
    if (interaction.kind !== "choose") throw new Error("Expected the frozen choice");
    matches("choice.json", choice);
    const left = interaction.options.find(option => option.key === "left");
    if (!left) throw new Error("Expected the frozen left branch");
    await reader.choose(choice.view.cursor, interaction.choice_point, [left.id]);
    const scene = await reader.inspect();
    matches("scene-transition.json", scene);
    if (scene.view.interaction.kind !== "advance") throw new Error("Expected the final dialogue");
    await reader.advance(scene.view.cursor, scene.view.interaction.dialogue);
    matches("finished.json", await reader.inspect());
  } finally {
    await reader.close();
  }
});

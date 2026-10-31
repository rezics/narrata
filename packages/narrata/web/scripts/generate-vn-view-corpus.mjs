import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
const [packageDirectory, outputDirectory] = process.argv.slice(2);
if (!packageDirectory || !outputDirectory) throw new Error("Expected installed package directory and new output directory under .temp/");
const output = resolve(root, outputDirectory), temporary = resolve(root, ".temp") + sep;
if (!output.startsWith(temporary) || existsSync(output)) throw new Error("Output must be a new directory under .temp/; frozen corpora are never overwritten");
const installed = resolve(packageDirectory);
const metadata = JSON.parse(readFileSync(resolve(installed, "package.json"), "utf8"));
const entry = metadata.exports?.["./vn"]?.import;
if (metadata.name !== "@rezics/narrata" || typeof entry !== "string" || !entry.startsWith("./")) throw new Error("Expected the installed public VN entry");
const { openVn } = await import(pathToFileURL(resolve(installed, entry)).href);
const pack = readFileSync(resolve(root, "fixtures/compat/nodes-vn-v1/story.narpack"));
const execution = "execution:0190f2a0000070008000000000000001";
const reader = await openVn({ pack, execution });
const snapshots = {};
try {
  snapshots["dialogue.json"] = await reader.inspect();
  for (let index = 0; index < 3; index++) {
    const current = await reader.inspect(), interaction = current.view.interaction;
    if (interaction.kind !== "advance") throw new Error("Expected three dialogue safe points");
    await reader.advance(current.view.cursor, interaction.dialogue);
  }
  const choice = await reader.inspect(), interaction = choice.view.interaction;
  if (interaction.kind !== "choose") throw new Error("Expected the frozen route choice");
  snapshots["choice.json"] = choice;
  const left = interaction.options.find(option => option.key === "left");
  if (!left) throw new Error("Expected the frozen left branch");
  await reader.choose(choice.view.cursor, interaction.choice_point, [left.id]);
  const scene = await reader.inspect();
  if (scene.view.interaction.kind !== "advance") throw new Error("Expected dialogue after the scene transition");
  snapshots["scene-transition.json"] = scene;
  await reader.advance(scene.view.cursor, scene.view.interaction.dialogue);
  const finished = await reader.inspect();
  if (finished.view.interaction.kind !== "finished") throw new Error("Expected the frozen ending");
  snapshots["finished.json"] = finished;
} finally {
  await reader.close();
}
const sha256 = bytes => createHash("sha256").update(bytes).digest("hex");
mkdirSync(output);
const artifacts = {};
for (const [name, snapshot] of Object.entries(snapshots)) {
  const bytes = JSON.stringify(snapshot, null, 2) + "\n";
  writeFileSync(resolve(output, name), bytes);
  artifacts[name] = { sha256: sha256(bytes) };
}
writeFileSync(resolve(output, "manifest.json"), JSON.stringify({
  corpus_version: 1,
  format: "ADR 0024 public @rezics/narrata/vn VnViewV1 JSON",
  package_version: metadata.version,
  source_program: "../nodes-vn-v1/story.narpack",
  source_program_sha256: sha256(pack),
  execution,
  artifact_id: snapshots["dialogue.json"].view.artifact_id,
  generation: "generate-vn-view-corpus.mjs through the independently installed public VN entry",
  artifacts,
}, null, 2) + "\n");
console.log(`Generated public VN View v1 corpus under ${output}`);

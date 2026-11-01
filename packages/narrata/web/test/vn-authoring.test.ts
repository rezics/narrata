import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import { loadVnAuthoring } from "@rezics/narrata/vn/server";
import { openVn } from "@rezics/narrata/vn";
import { shortStorySource } from "@rezics/narrata/testing";
import { execution } from "./fixture.js";

const text = (path: string): unknown => JSON.parse(readFileSync(resolve(import.meta.dirname, "../../../../products/visual-novel-demo", path), "utf8"));
const project = text("project.json");
const source = text("packages/main.json");
const api = loadVnAuthoring();
const object = (value: unknown): Record<string, unknown> => {
  if (value === null || typeof value !== "object" || Array.isArray(value)) throw new Error("Expected fixture object");
  return value as Record<string, unknown>;
};

function drafts(packageSource: unknown = source) {
  const value = api.then(authoring => {
    const drafts = authoring.createVnDrafts();
    const split = authoring.splitVnSource(project, [["main", packageSource]]);
    if (!split.complete || split.diagnostics.some(item => item.severity === "error")) throw new Error(JSON.stringify(split.diagnostics));
    for (const record of split.records) drafts.add(record.bytes);
    return drafts;
  });
  return value;
}

test("VN source checks and previews one immutable source-format-3 candidate", async () => {
  const authoring = await api;
  const input = await drafts();
  const checked = input.check();
  expect(checked.complete).toBe(true);
  expect(checked.diagnostics).toEqual([]);
  expect(checked.records.length).toBeGreaterThan(2);
  const candidate = input.preview();
  const repeated = input.preview();
  try {
    expect(candidate.compiled).toBe(true);
    expect(candidate.report.artifact_id).toMatch(/^artifact:[0-9a-f]{64}$/);
    expect(candidate.report.lock).toBeDefined();
    expect(candidate.report.artifact_id).toBe(repeated.report.artifact_id);
    expect(candidate.pack()).toEqual(repeated.pack());
    expect(candidate.manifest()).toBeInstanceOf(Uint8Array);
    expect(candidate.names()).toBeDefined();
    for (const id of candidate.report.objects) {
      expect(candidate.object(id)).toEqual(repeated.object(id));
    }
    const lock = candidate.report.lock;
    if (!lock) throw new Error("Candidate lock expected");
    const mismatched = input.preview({ lock: { ...lock, artifact_id: `artifact:${"0".repeat(64)}` } });
    try {
      expect(mismatched.compiled).toBe(false);
      expect(mismatched.report.diagnostics.map(item => item.code)).toContain("lock_mismatch");
    } finally { mismatched.free(); }
    const reader = await openVn({ pack: candidate.pack(), execution });
    try { expect((await reader.inspect()).view.interaction.kind).toBe("advance"); }
    finally { await reader.close(); }

    const changed = JSON.parse(JSON.stringify(source)) as unknown;
    const graphs = object(object(changed).graphs);
    const nodes = object(object(graphs.start).nodes);
    const river = object(nodes.river);
    const data = object(river.data);
    const background = object(data.background);
    background.revision = "2";
    const edited = await drafts(changed);
    const editedCheck = edited.check();
    expect(editedCheck.complete).toBe(true);
    const next = edited.preview();
    try {
      expect(next.compiled).toBe(true);
      expect(next.report.artifact_id).not.toBe(candidate.report.artifact_id);
      expect(candidate.pack()).toEqual(repeated.pack());
    } finally { next.free(); edited.free(); }
  } finally { candidate.free(); repeated.free(); input.free(); }
  expect(() => authoring.checkVnRecord(new Uint8Array([0]))).not.toThrow();
  const structural = authoring.checkVnRecord(new TextEncoder().encode(JSON.stringify({ format_version: 1, kind: "project", payload: {} })));
  expect(structural.record).toBeUndefined();
  expect(structural.diagnostics.some(item => item.severity === "error")).toBe(true);
});

test("a structurally valid record set with invalid VN semantics cannot compile", async () => {
  const bad = JSON.parse(JSON.stringify(source)) as unknown;
  const graphs = object(object(bad).graphs);
  const nodes = object(object(graphs.start).nodes);
  const opening = object(nodes["mira-opening"]);
  delete object(opening.data).line;
  const input = await drafts(bad);
  try {
    const check = input.check();
    expect(check.complete).toBe(true);
    expect(check.diagnostics.some(item => item.severity === "error")).toBe(true);
    const candidate = input.preview();
    try {
      expect(candidate.compiled).toBe(false);
      expect(candidate.report.diagnostics.some(item => item.severity === "error")).toBe(true);
    } finally { candidate.free(); }
  } finally { input.free(); }
});

test("the VN authoring entry rejects a source-format-2 Gamebook snapshot", async () => {
  const authoring = await api;
  const gamebook = shortStorySource();
  const split = authoring.splitVnSource(gamebook.manifest, gamebook.packages);
  const input = authoring.createVnDrafts();
  try {
    for (const record of split.records) input.add(record.bytes);
    expect(input.check().diagnostics.some(item => item.severity === "error")).toBe(true);
    const candidate = input.preview();
    try { expect(candidate.compiled).toBe(false); }
    finally { candidate.free(); }
  } finally { input.free(); }
});

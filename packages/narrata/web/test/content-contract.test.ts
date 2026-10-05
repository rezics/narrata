import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import { Ajv2020 } from "ajv/dist/2020.js";
import { compileFromFile } from "json-schema-to-typescript";
import { MockRezicsContent, mountainLetterData, mountainLetterKeys } from "../src/testing/index.js";
import type { ResolveRequest } from "../src/content/index.js";

const root = resolve(import.meta.dirname, "../../../..");
const schemas = resolve(root, "packages/narrata/nodes/schemas");
const ajv = new Ajv2020({ strict: true, validateFormats: false });
function validator(name: string) { return ajv.compile(JSON.parse(readFileSync(resolve(schemas, `${name}.schema.json`), "utf8"))); }

test("all content declarations match their authoritative Rust schemas", async () => {
  for (const [schema, file] of [["content-resolve-request", "request"], ["content-resolution", "resolution"], ["content-outline", "outline"]]) {
    const generated = await compileFromFile(resolve(schemas, `${schema}.schema.json`), {
      bannerComment: "/* Generated from the Rust content JSON Schema; checked by content contract tests. */", additionalProperties: false,
    });
    expect(readFileSync(resolve(root, `packages/narrata/web/src/content/${file}.ts`), "utf8")).toBe(generated);
  }
});

test("requests keep old language-only contexts and results keep the three outcomes", async () => {
  const key = mountainLetterKeys["main.ledger"];
  if (!key) throw new Error("Missing sample");
  const request: ResolveRequest = { context: { languages: ["en"], realization: "en", viewer: "host-reader:opaque" }, items: [
    { content: { unit: { provider: "rezics", key }, first: "l1", last: "l2" } },
    { content: { provider: "rezics", key: "missing" } },
    { content: { unit: { provider: "rezics", key }, first: "missing" } },
  ] };
  const validateRequest = validator("content-resolve-request");
  expect(validateRequest(request)).toBe(true);
  expect(validateRequest({ ...request, context: { languages: ["en"] } })).toBe(true);
  expect(validateRequest({ ...request, context: { realization: null, viewer: null } })).toBe(true);
  expect(validateRequest({ ...request, context: { viewer: {} } })).toBe(false);
  expect(validateRequest({ ...request, context: { extra: true } })).toBe(false);
  const mock = new MockRezicsContent(mountainLetterData());
  const results = await mock.resolve(request);
  expect(results.map(result => result.status)).toEqual(["ok", "unavailable", "incompatible"]);
  const validateResults = validator("content-resolution");
  expect(validateResults(results)).toBe(true);
  expect(validateResults([{ status: "not_found" }])).toBe(false);
  expect(validateResults([{ status: "unavailable", reason: "permission_denied" }])).toBe(false);
  expect(validator("content-outline")(mock.outline())).toBe(true);
});

test("the mock's complete original data retains the example's body text and block IDs", () => {
  const original: unknown = JSON.parse(readFileSync(resolve(root, "products/gamebook-demo/content/zh-Hans.json"), "utf8"));
  const data = mountainLetterData();
  const reconstructed = Object.fromEntries(Object.entries(mountainLetterKeys).map(([name, key]) => {
    const document = data.occurrences[key]?.realizations.original;
    const label = data.labels[key]?.realizations.original;
    if (document) return [name, { blocks: document.blocks.map(block => {
      if (block.type !== "paragraph") throw new Error("Unexpected sample marker");
      return { id: block.attrs.id, text: block.text };
    }) }];
    if (!label) throw new Error("Missing sample original");
    return [name, { text: label.text }];
  }));
  expect(original).toEqual({ format_version: 1, provider: "local", language: "zh-Hans", entries: reconstructed });
});

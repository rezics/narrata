import Ajv2020 from "ajv/dist/2020.js";
import { z } from "zod";
import schema from "./generated/book-view.schema.json" with { type: "json" };
import type { BookView, ContentRef, Segment, ViewScalar } from "./generated/book-view";

// The static type and runtime schema are generated from the same Rust BookView definition.
const validateBook = new Ajv2020({ strict: true, validateFormats: false }).compile<BookView>(schema);
export function checkedBook(value: unknown): BookView {
  if (!validateBook(value)) throw new Error("引擎返回了无法识别的界面数据，请重新构建运行时。");
  return value;
}
export function decodeBook(text: string): BookView { const value: unknown = JSON.parse(text); return checkedBook(value); }

export type Content = ContentRef | Segment;
export type Args = Record<string, ViewScalar>;

function stable(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${stable((value as Record<string, unknown>)[key])}`).join(",")}}`;
  return JSON.stringify(value) ?? "null";
}
/** The key a resolved text is sent under: the content with the arguments it was formatted with. */
export function contentKey(content: Content, args: Args = {}): string { return stable([content, args]); }

/** A resolved short text is one block; an unresolved item keeps its reason. */
export const resolvedSchema = z.union([
  z.object({ ok: z.literal(true), blocks: z.array(z.string()) }).strict(),
  z.object({ ok: z.literal(false), reason: z.string() }).strict(),
]);
export type Resolved = z.infer<typeof resolvedSchema>;

const MiB = 1024 * 1024;
export const limits = { pack: 16 * MiB, content: 16 * MiB, save: 8 * MiB, r1Save: 4 * MiB } as const;
const id = z.number().int().nonnegative();
const bytes = z.instanceof(Uint8Array).refine(value => value.byteLength <= limits.pack, "构件超过 16 MiB");
export const requestSchema = z.discriminatedUnion("kind", [
  z.object({ id, kind: z.literal("boot") }).strict(),
  z.object({ id, kind: z.literal("choose"), expected: z.string(), choice_point: z.string(), options: z.array(z.string()).max(128) }).strict(),
  z.object({ id, kind: z.literal("checkout"), commit: z.string() }).strict(),
  z.object({ id, kind: z.literal("open"), pack: bytes, content: z.array(z.string().max(limits.content)).min(1).max(8) }).strict(),
  z.object({ id, kind: z.literal("restore"), save: z.string().max(limits.save) }).strict(),
  z.object({ id, kind: z.literal("restart") }).strict(),
  z.object({ id, kind: z.literal("export_save") }).strict(),
  z.object({ id, kind: z.literal("export_pack") }).strict(),
  z.object({ id, kind: z.literal("export_content") }).strict(),
]);
export type Request = z.infer<typeof requestSchema>;
export type Command = Request extends infer R ? R extends Request ? Omit<R, "id"> : never : never;

const bookValue = z.unknown().transform((value, ctx) => {
  try { return checkedBook(value); } catch { ctx.addIssue({ code: "custom", message: "Invalid BookView" }); return z.NEVER; }
});
export const replySchema = z.discriminatedUnion("kind", [
  z.object({ id, kind: z.literal("view"), book: bookValue, texts: z.record(z.string(), resolvedSchema), saved_at: z.iso.datetime().nullable(), warning: z.string().nullable() }).strict(),
  z.object({ id, kind: z.literal("file"), files: z.array(z.object({ data: z.union([z.string(), z.instanceof(Uint8Array)]), filename: z.string(), type: z.string() }).strict()) }).strict(),
  z.object({ id, kind: z.literal("error"), message: z.string() }).strict(),
]);
export type Reply = z.infer<typeof replySchema>;
export type ViewReply = Extract<Reply, { kind: "view" }>;

export function errorMessage(error: unknown): string { return error instanceof Error ? error.message : String(error); }

import Ajv2020 from "ajv/dist/2020.js";
import { z } from "zod";
import schema from "./generated/book-view.schema.json" with { type: "json" };
import type { BookView } from "./generated/book-view";

// The static type and runtime schema are generated from the same Rust BookView definition.
const validateBook = new Ajv2020({ strict: true, validateFormats: false }).compile<BookView>(schema);
export function checkedBook(value: unknown): BookView {
  if (!validateBook(value)) throw new Error("引擎返回了无法识别的界面数据，请重新构建运行时。");
  return value;
}
export function decodeBook(text: string): BookView { const value: unknown = JSON.parse(text); return checkedBook(value); }

const id = z.number().int().nonnegative();
const text = z.string().max(4 * 1024 * 1024);
export const requestSchema = z.discriminatedUnion("kind", [
  z.object({ id, kind: z.literal("boot") }).strict(),
  z.object({ id, kind: z.literal("select"), expected: z.string(), action: z.string() }).strict(),
  z.object({ id, kind: z.literal("checkout"), commit: z.string() }).strict(),
  z.object({ id, kind: z.literal("open"), source: text }).strict(),
  z.object({ id, kind: z.literal("restore"), save: text }).strict(),
  z.object({ id, kind: z.literal("restart") }).strict(),
  z.object({ id, kind: z.literal("export_save") }).strict(),
  z.object({ id, kind: z.literal("export_project") }).strict(),
]);
export type Request = z.infer<typeof requestSchema>;
export type Command = Request extends infer R ? R extends Request ? Omit<R, "id"> : never : never;

const bookValue = z.unknown().transform((value, ctx) => {
  try { return checkedBook(value); } catch { ctx.addIssue({ code: "custom", message: "Invalid BookView" }); return z.NEVER; }
});
export const replySchema = z.discriminatedUnion("kind", [
  z.object({ id, kind: z.literal("view"), book: bookValue, saved_at: z.iso.datetime().nullable(), warning: z.string().nullable() }).strict(),
  z.object({ id, kind: z.literal("file"), text: z.string(), filename: z.string() }).strict(),
  z.object({ id, kind: z.literal("error"), message: z.string() }).strict(),
]);
export type Reply = z.infer<typeof replySchema>;
export type ViewReply = Extract<Reply, { kind: "view" }>;

export function errorMessage(error: unknown): string { return error instanceof Error ? error.message : String(error); }

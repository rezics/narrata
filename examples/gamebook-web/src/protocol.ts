import { checkedBook, decodeBook, contentItemKey } from "@rezics/narrata";
import { z } from "zod";
import type { ContentRef, Segment, ViewScalar } from "@rezics/narrata";

export { checkedBook, decodeBook };

export type Content = ContentRef | Segment;
export type Args = Record<string, ViewScalar>;

/** The key a resolved text is sent under: the content with the arguments it was formatted with. */
export function contentKey(content: Content, args: Args = {}): string { return contentItemKey({ content, args }); }

const contentRefSchema = z.object({ provider: z.string(), key: z.string() }).strict();
const segmentSchema = z.object({ unit: contentRefSchema, first: z.string().nullable().optional(), last: z.string().nullable().optional() }).strict();
const scalarSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("bool"), value: z.boolean() }).strict(),
  z.object({ type: z.literal("int"), value: z.string() }).strict(),
  z.object({ type: z.literal("text"), value: z.string() }).strict(),
  z.object({ type: z.literal("ref"), value: contentRefSchema }).strict(),
]);
const resolveRequestSchema = z.object({
  context: z.object({ languages: z.array(z.string()).optional(), realization: z.string().nullable().optional(), viewer: z.string().nullable().optional() }).strict(),
  items: z.array(z.object({ content: z.union([segmentSchema, contentRefSchema]), args: z.record(z.string(), scalarSchema).optional() }).strict()).max(4096),
}).strict();
export const resolutionSchema = z.array(z.discriminatedUnion("status", [
  z.object({ status: z.literal("ok"), revision: z.string(), payload: z.union([
    z.object({ text: z.string() }).strict(),
    z.object({ blocks: z.array(z.object({ id: z.string(), text: z.string() }).strict()) }).strict(),
  ]) }).strict(),
  z.object({ status: z.literal("unavailable") }).strict(),
  z.object({ status: z.literal("incompatible"), reason: z.string() }).strict(),
]));

/** Debug-panel text uses one block per paragraph; unresolved items retain only their status. */
export const resolvedSchema = z.union([
  z.object({ ok: z.literal(true), blocks: z.array(z.string()) }).strict(),
  z.object({ ok: z.literal(false), reason: z.enum(["unavailable", "incompatible"]) }).strict(),
]);
export type Resolved = z.infer<typeof resolvedSchema>;

const MiB = 1024 * 1024;
export const limits = { pack: 16 * MiB, content: 16 * MiB, save: 64 * MiB, r1Save: 4 * MiB } as const;
const id = z.number().int().nonnegative();
const bytes = z.instanceof(Uint8Array).refine(value => value.byteLength <= limits.pack, "构件超过 16 MiB");
export const requestSchema = z.discriminatedUnion("kind", [
  z.object({ id, kind: z.literal("boot") }).strict(),
  z.object({ id, kind: z.literal("reload") }).strict(),
  z.object({ id, kind: z.literal("lookahead") }).strict(),
  z.object({ id, kind: z.literal("resolve"), request: resolveRequestSchema }).strict(),
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
  z.object({ id, kind: z.literal("resolved"), results: resolutionSchema }).strict(),
  z.object({ id, kind: z.literal("units"), units: z.array(contentRefSchema) }).strict(),
  z.object({ id, kind: z.literal("file"), files: z.array(z.object({ data: z.union([z.string(), z.instanceof(Uint8Array)]), filename: z.string(), type: z.string() }).strict()) }).strict(),
  z.object({ id, kind: z.literal("error"), message: z.string(), superseded: z.boolean().optional() }).strict(),
]);
export type Reply = z.infer<typeof replySchema>;
export type ViewReply = Extract<Reply, { kind: "view" }>;

export function errorMessage(error: unknown): string { return error instanceof Error ? error.message : String(error); }

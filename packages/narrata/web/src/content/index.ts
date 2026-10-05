import type { BookView, VariableView } from "../generated/book-view.js";
import type { Content, ResolveContext, ResolveItem, ResolveRequest, ViewScalar } from "./request.js";
import type { Resolution } from "./resolution.js";

export type { Content, ResolveContext, ResolveItem, ResolveRequest } from "./request.js";
export type { Resolution, Payload, TextBlock } from "./resolution.js";
export type { ContentOutline, UnitOutline } from "./outline.js";

/** Host implementation. Results correspond to request items in order, including failures. */
export interface ContentResolver {
  resolve(request: ResolveRequest): Promise<Resolution[]>;
}

/** Identity includes segment bounds and arguments; deduplication must not lose either. */
export function contentItemKey(item: ResolveItem): string {
  const content = item.content;
  const reference = "unit" in content ? content.unit : content;
  const bounds = "unit" in content ? [content.first ?? null, content.last ?? null] : null;
  const args = Object.entries(item.args ?? {}).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
    .map(([name, scalar]) => [name, scalar.type, scalar.type === "ref" ? [scalar.value.provider, scalar.value.key] : scalar.value]);
  return JSON.stringify([reference.provider, reference.key, bounds, args]);
}

/** References in the reading page and revealed session UI, never graph-analysis nodes. */
export function collectScreenContent(book: BookView): ResolveItem[] {
  const items = new Map<string, ResolveItem>();
  const add = (content: Content | null | undefined, args?: Record<string, ViewScalar>) => {
    if (content) {
      const item: ResolveItem = { content, ...(args ? { args } : {}) };
      items.set(contentItemKey(item), structuredClone(item));
    }
  };
  const variables = (values: VariableView[]) => {
    for (const value of values) { add(value.label); if (value.value.type === "ref") add(value.value.value); }
  };
  add(book.view.product.title);
  for (const item of book.page) add(item.content, item.args);
  const interaction = book.view.interaction;
  if (interaction.kind === "choose") {
    for (const option of interaction.options) { add(option.label, interaction.args); add(option.reason, interaction.args); }
  } else { add(interaction.title); add(interaction.body); }
  variables(book.view.shared);
  for (const frame of book.view.frames) { variables(frame.parameters); variables(frame.locals); }
  for (const entry of book.view.history) add(entry.title);
  return [...items.values()];
}

/** Split at the host's item limit; every batch receives the same context values. */
export function contentRequests(items: readonly ResolveItem[], context: ResolveContext, maxItems: number): ResolveRequest[] {
  if (!Number.isSafeInteger(maxItems) || maxItems < 1) throw new RangeError("maxItems must be a positive safe integer");
  const requests: ResolveRequest[] = [];
  for (let offset = 0; offset < items.length; offset += maxItems) {
    requests.push({ context: structuredClone(context), items: structuredClone(items.slice(offset, offset + maxItems)) });
  }
  return requests;
}

/** One screen context, bounded batches, stable item/result ordering. Network failures reject. */
export async function resolveContent(
  resolver: ContentResolver, items: readonly ResolveItem[], context: ResolveContext, maxItems: number,
): Promise<Resolution[]> {
  const results: Resolution[] = [];
  for (const request of contentRequests(items, context, maxItems)) {
    const batch = await resolver.resolve(request);
    if (batch.length !== request.items.length) throw new Error("Content resolver returned a different result count");
    results.push(...batch);
  }
  return results;
}

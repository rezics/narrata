import { contentItemKey, type ContentOutline, type ContentResolver, type ResolveContext, type ResolveItem, type ResolveRequest, type Resolution } from "./index.js";

/** Native values stay with the host; only their text fallback enters a ReaderScreen. */
export interface NativeTextBlock<Value> {
  readonly id: string;
  readonly text: string;
  readonly value: Value;
  readonly choicePoint?: never;
}
export interface NativeChoiceMarker {
  readonly id: string;
  readonly choicePoint: string;
  readonly text?: never;
  readonly value?: never;
}
export type NativeBlock<Value> = NativeTextBlock<Value> | NativeChoiceMarker;
/** A host-validated, structured-cloneable snapshot. Revisions identify immutable content. */
export interface NativeDocument<Value> {
  readonly revision: string;
  readonly blocks: readonly NativeBlock<Value>[];
}
export interface NativeSlice<Value> {
  readonly revision: string;
  readonly blocks: readonly NativeTextBlock<Value>[];
}
/** This is an in-process host callback result, not a new content-resolution wire format. */
export type NativeContentResult<Value> = Resolution | { readonly status: "document"; readonly document: NativeDocument<Value> };
export interface NativeContentOptions<Value> {
  /** Resolve labels normally and body units to a single snapshot, using the supplied context. */
  readonly resolve: (request: ResolveRequest) => Promise<readonly NativeContentResult<Value>[]>;
  readonly maxCacheEntries?: number;
  readonly maxCacheBlocks?: number;
}
type RenderResolution = Resolution | { readonly status: "incompatible" };
export interface NativeContentAdapter<Value> extends ContentResolver {
  /** A miss is safe: render the standard payload instead. This never fetches newer content. */
  lookup(item: ResolveItem, context: ResolveContext, resolution: RenderResolution): NativeSlice<Value> | undefined;
  /** Clear on account/permission changes. Pending requests cannot repopulate the cache. */
  clear(): void;
}

function checkedDocument<Value>(document: NativeDocument<Value>): NativeDocument<Value> {
  const snapshot = structuredClone(document);
  if (typeof snapshot.revision !== "string" || !Array.isArray(snapshot.blocks)) throw new Error("Invalid native document snapshot");
  const ids = new Set<string>();
  for (const block of snapshot.blocks) {
    if (typeof block.id !== "string" || !/^[ -~]{1,128}$/.test(block.id) || ids.has(block.id)) throw new Error("Invalid or duplicate native block anchor");
    ids.add(block.id);
    if (block.choicePoint !== undefined) {
      if (!/^choice-point:[0-9a-f]{32}$/.test(block.choicePoint) || "value" in block || "text" in block) throw new Error("Invalid native choice marker");
    } else if (typeof block.text !== "string" || !("value" in block)) throw new Error("Native block needs text fallback and value");
  }
  return snapshot;
}

/** Derive an original-language outline from the same snapshots used by a publication job. */
export function nativeContentOutline<Value>(provider: string, units: Readonly<Record<string, NativeDocument<Value>>>): ContentOutline {
  if (!/^[a-z0-9-]{1,32}$/.test(provider)) throw new Error("Invalid content provider");
  return { format_version: 1, provider, units: Object.fromEntries(Object.entries(units).map(([key, document]) => {
    if (!key || new TextEncoder().encode(key).length > 256 || /[\u0000-\u001f\u007f-\u009f]/u.test(key)) throw new Error("Invalid content unit key");
    const snapshot = checkedDocument(document);
    return [key, { blocks: snapshot.blocks.map(block => block.id), markers: Object.fromEntries(snapshot.blocks.flatMap(block =>
      block.choicePoint === undefined ? [] : [[block.id, block.choicePoint]])) }];
  })) };
}

function cacheKey(item: ResolveItem, context: ResolveContext, revision: string): string {
  return JSON.stringify([contentItemKey(item), context.languages ?? null, context.realization ?? null, context.viewer ?? null, revision]);
}
function positive(value: number | undefined, fallback: number): number {
  const result = value ?? fallback;
  if (!Number.isSafeInteger(result) || result < 1) throw new RangeError("Native cache budgets must be positive safe integers");
  return result;
}

/** Checked inclusive slicing, standard fallbacks and a bounded host-owned native cache. */
export function createNativeContentAdapter<Value>(options: NativeContentOptions<Value>): NativeContentAdapter<Value> {
  const resolve = options.resolve;
  const maxEntries = positive(options.maxCacheEntries, 64), maxBlocks = positive(options.maxCacheBlocks, 4096);
  const cache = new Map<string, NativeSlice<Value>>();
  let blocks = 0, generation = 0;
  const remove = (key: string) => { const entry = cache.get(key); if (entry) { blocks -= entry.blocks.length; cache.delete(key); } };
  return {
    async resolve(request) {
      const input = structuredClone(request), token = generation;
      // The callback may mutate its argument; cache identity uses the untouched request.
      const results = await resolve(structuredClone(input));
      if (results.length !== input.items.length) throw new Error("Native resolver returned a different result count");
      return results.map((result, index): Resolution => {
        if (result.status !== "document") return structuredClone(result);
        try {
          const snapshot = checkedDocument(result.document), item = input.items[index]!;
          const segment = "unit" in item.content ? item.content : undefined;
          const first = segment?.first == null ? 0 : snapshot.blocks.findIndex(block => block.id === segment.first);
          const last = segment?.last == null ? snapshot.blocks.length - 1 : snapshot.blocks.findIndex(block => block.id === segment.last);
          if (first < 0 || (last < 0 && segment?.last != null)) throw new Error("Missing native segment endpoint");
          if (snapshot.blocks.length && first > last) throw new Error("Reversed native segment endpoints");
          const selected = snapshot.blocks.slice(first, last + 1).filter((block): block is NativeTextBlock<Value> => block.choicePoint === undefined);
          const slice = { revision: snapshot.revision, blocks: selected };
          if (token === generation && selected.length <= maxBlocks) {
            const key = cacheKey(item, input.context, snapshot.revision);
            remove(key); cache.set(key, slice); blocks += selected.length;
            while (cache.size > maxEntries || blocks > maxBlocks) remove(cache.keys().next().value!);
          }
          return { status: "ok", revision: snapshot.revision, payload: { blocks: selected.map(block => ({ id: block.id, text: block.text })) } };
        } catch (error) {
          return { status: "incompatible", reason: error instanceof Error ? error.message : "Invalid native document" };
        }
      });
    },
    lookup(item, context, resolution) {
      if (resolution.status !== "ok") return undefined;
      const key = cacheKey(item, context, resolution.revision), slice = cache.get(key);
      if (!slice) return undefined;
      cache.delete(key); cache.set(key, slice);
      // A caller cannot mutate either the host's original snapshot or the cached copy.
      return structuredClone(slice);
    },
    clear() { generation++; cache.clear(); blocks = 0; },
  };
}

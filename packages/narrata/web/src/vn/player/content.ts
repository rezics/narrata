import { contentItemKey, contentRequests } from "../../content/index.js";
import type { Content, ResolveContext, ResolveItem, ResolveRequest, Resolution, TextBlock } from "../../content/index.js";
import type { ContentRef, Segment, VnViewV1, ViewScalar } from "../../generated/vn-view.js";

export interface VnTextResolver {
  resolve(request: ResolveRequest, signal: AbortSignal): Promise<readonly Resolution[]>;
}

export class VnTextUnavailableError extends Error {
  constructor() {
    super("Current VN text is unavailable.");
    this.name = "VnTextUnavailableError";
  }
}

export function collectVnText(view: VnViewV1): ResolveItem[] {
  const items = new Map<string, ResolveItem>();
  const add = (content: Content | null | undefined, args?: Record<string, ViewScalar>) => {
    if (!content) return;
    const item: ResolveItem = { content, ...(args ? { args } : {}) };
    items.set(contentItemKey(item), item);
  };

  for (const item of view.view.presentation) add(item.content, item.args);
  const interaction = view.view.interaction;
  switch (interaction.kind) {
    case "advance":
      add(interaction.line);
      add(interaction.speaker);
      break;
    case "choose":
      for (const option of interaction.options) {
        add(option.label, interaction.args);
        add(option.reason, interaction.args);
      }
      break;
    case "finished":
      add(interaction.title);
      add(interaction.body);
      break;
  }
  return [...items.values()];
}

export async function resolveVnText(
  view: VnViewV1,
  resolver: VnTextResolver,
  context: ResolveContext,
  signal: AbortSignal,
  maxItems = 64,
): Promise<ReadonlyMap<string, string>> {
  const items = collectVnText(view);
  if (items.length > maxItems) throw new VnTextUnavailableError();
  const requests = contentRequests(items, context, maxItems);
  const batches = await Promise.all(requests.map(async request => {
    if (signal.aborted) throw abortError();
    const values = await awaitAbortable(resolver.resolve(request, signal), signal);
    if (signal.aborted) throw abortError();
    if (!Array.isArray(values) || values.length !== request.items.length) throw new VnTextUnavailableError();
    return values;
  }));
  const resolutions = batches.flat();
  if (resolutions.length !== items.length) throw new VnTextUnavailableError();
  const resolved = new Map<string, string>();
  for (const [index, item] of items.entries()) {
    const resolution: unknown = resolutions[index];
    const text = resolutionText(item.content, resolution);
    resolved.set(contentItemKey(item), text);
  }
  return resolved;
}

export function vnText(
  texts: ReadonlyMap<string, string>,
  content: Content | null | undefined,
  args?: Record<string, ViewScalar>,
): string | undefined {
  if (!content) return undefined;
  return texts.get(contentItemKey({ content, ...(args ? { args } : {}) }));
}

export function contentReferenceKey(reference: ContentRef): string {
  return JSON.stringify([reference.provider, reference.key]);
}

function resolutionText(content: Content, value: unknown): string {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new VnTextUnavailableError();
  const result = value as Record<string, unknown>;
  if (result.status !== "ok") throw new VnTextUnavailableError();
  if (typeof result.revision !== "string" || result.revision.length === 0) throw new VnTextUnavailableError();
  if (typeof result.payload !== "object" || result.payload === null || Array.isArray(result.payload)) throw new VnTextUnavailableError();
  const payload = result.payload as Record<string, unknown>;
  if (typeof payload.text === "string") return payload.text;
  if (!Array.isArray(payload.blocks)) throw new VnTextUnavailableError();
  const blocks: TextBlock[] = payload.blocks.map(block => {
    if (typeof block !== "object" || block === null || Array.isArray(block)) throw new VnTextUnavailableError();
    const row = block as Record<string, unknown>;
    if (typeof row.id !== "string" || typeof row.text !== "string") throw new VnTextUnavailableError();
    return { id: row.id, text: row.text };
  });
  if ("unit" in content) return segmentText(content, blocks);
  return blocks.map(block => block.text).join("\n");
}

function segmentText(segment: Segment, blocks: readonly TextBlock[]): string {
  const firstIndex = segment.first == null ? 0 : blocks.findIndex(block => block.id === segment.first);
  const lastIndex = segment.last == null ? blocks.length - 1 : blocks.findIndex(block => block.id === segment.last);
  if (firstIndex < 0 || lastIndex < firstIndex) throw new VnTextUnavailableError();
  return blocks.slice(firstIndex, lastIndex + 1).map(block => block.text).join("\n");
}

function abortError(): DOMException {
  return new DOMException("VN resource work was cancelled.", "AbortError");
}

function awaitAbortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let settled = false;
    const cleanup = () => signal.removeEventListener("abort", abort);
    const abort = () => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(abortError());
    };
    if (signal.aborted) {
      settled = true;
      void promise.catch(() => undefined);
      reject(abortError());
      return;
    }
    signal.addEventListener("abort", abort, { once: true });
    promise.then(value => {
      if (settled || signal.aborted) {
        settled = true;
        cleanup();
        return;
      }
      settled = true;
      cleanup();
      resolve(value);
    }, error => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error);
    });
  });
}

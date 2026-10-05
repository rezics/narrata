import type { ChoicePointId } from "../generated/book-view.js";
import type { ContentOutline, ContentResolver, ResolveContext, ResolveItem, ResolveRequest, Resolution, Payload } from "../content/index.js";

/** A flat REZICS document. Translations retain the original attrs.id anchors. */
export type MockBlock =
  | { type: "paragraph"; attrs: { id: string }; text: string }
  | { type: "narrata-choice"; attrs: { id: string; choice_point: ChoicePointId } };
export interface MockDocument { language: string; revision: string; blocks: MockBlock[] }
export interface MockText { language: string; revision: string; text: string }
export interface MockOccurrence { original: string; realizations: Record<string, MockDocument> }
export interface MockLabel { original: string; realizations: Record<string, MockText> }
export interface MockRezicsData { occurrences: Record<string, MockOccurrence>; labels: Record<string, MockLabel> }

function selected<T extends { language: string }>(entry: { original: string; realizations: Record<string, T> }, context: ResolveContext): T {
  const original = Object.hasOwn(entry.realizations, entry.original) ? entry.realizations[entry.original] : undefined;
  if (!original) throw new Error("Mock content has no original realization");
  if (context.realization) return (Object.hasOwn(entry.realizations, context.realization) ? entry.realizations[context.realization] : undefined) ?? original;
  for (const language of context.languages ?? []) {
    let range = language;
    while (range) {
      const match = Object.values(entry.realizations).find(value => value.language.toLowerCase() === range.toLowerCase());
      if (match) return match;
      const end = range.lastIndexOf("-");
      range = end < 0 ? "" : range.slice(0, end);
      if (range.length >= 2 && range.at(-2) === "-") range = range.slice(0, -2);
    }
  }
  return original;
}

/** Test-only host content provider; no production transport, storage or permission policy. */
export class MockRezicsContent implements ContentResolver {
  private readonly data: MockRezicsData;
  private readonly withdrawn = new Set<string>();
  private readonly readers = new Map<string, Set<string>>();

  constructor(data: MockRezicsData, readonly maxItems = 64) {
    if (!Number.isSafeInteger(maxItems) || maxItems < 1) throw new RangeError("Invalid mock batch limit");
    this.data = structuredClone(data);
    for (const [key, entry] of Object.entries(this.data.occurrences)) {
      if (Object.hasOwn(this.data.labels, key)) throw new Error("Occurrence and label keys collide");
      const original = entry.realizations[entry.original];
      if (!original) throw new Error("Mock occurrence has no original realization");
      const anchors = original.blocks.map(block => block.attrs.id);
      if (new Set(anchors).size !== anchors.length) throw new Error("Duplicate original block ID");
      for (const document of Object.values(entry.realizations)) {
        let previous = -1;
        for (const block of document.blocks) {
          const index = anchors.indexOf(block.attrs.id);
          if (index <= previous) throw new Error("Translation must retain original block IDs and order");
          previous = index;
          const source = original.blocks[index];
          if (source?.type !== block.type || (source.type === "narrata-choice" && block.type === "narrata-choice"
            && source.attrs.choice_point !== block.attrs.choice_point)) throw new Error("Translation changed a choice marker");
        }
      }
    }
    for (const label of Object.values(this.data.labels)) {
      if (!label.realizations[label.original]) throw new Error("Mock label has no original realization");
    }
  }

  withdraw(key: string): void { this.withdrawn.add(key); }
  /** Omitted restriction is public; an empty list grants nobody access. */
  restrict(key: string, viewers: readonly string[]): void { this.readers.set(key, new Set(viewers)); }

  async resolve(request: ResolveRequest): Promise<Resolution[]> {
    if (request.items.length > this.maxItems) throw new RangeError("Mock content batch limit exceeded");
    return request.items.map(item => this.resolveItem(item, request.context));
  }

  private resolveItem(item: ResolveItem, context: ResolveContext): Resolution {
    const segment = "unit" in item.content ? item.content : undefined;
    const reference = "unit" in item.content ? item.content.unit : item.content;
    const key = reference.key;
    const readers = this.readers.get(key);
    if (reference.provider !== "rezics" || this.withdrawn.has(key)
      || (readers && (!context.viewer || !readers.has(context.viewer)))) return { status: "unavailable" };
    const occurrence = Object.hasOwn(this.data.occurrences, key) ? this.data.occurrences[key] : undefined;
    const label = Object.hasOwn(this.data.labels, key) ? this.data.labels[key] : undefined;
    if (!occurrence && !label) return { status: "unavailable" };
    const format = (text: string): string => text.replace(/{{|}}|{([^{}]+)}|[{}]/g, (match: string, name: string | undefined) => {
      if (match === "{{" || match === "}}") return match[0] ?? "";
      if (!name) throw new Error("Unmatched template brace");
      const value = item.args && Object.hasOwn(item.args, name) ? item.args[name] : undefined;
      if (!value) throw new Error(`Missing argument ${name}`);
      if (value.type !== "ref") return String(value.value);
      const result = this.resolveItem({ content: value.value }, context);
      if (result.status !== "ok" || !("text" in result.payload)) throw new Error(`Argument ${name} does not resolve to text`);
      return result.payload.text;
    });
    try {
      let payload: Payload;
      let revision: string;
      if (occurrence) {
        const document = selected(occurrence, context);
        const start = segment?.first == null ? 0 : document.blocks.findIndex(block => block.attrs.id === segment.first);
        const end = segment?.last == null ? document.blocks.length - 1 : document.blocks.findIndex(block => block.attrs.id === segment.last);
        if (start < 0 || (end < 0 && segment?.last != null)) throw new Error("Missing segment endpoint");
        if (document.blocks.length && start > end) throw new Error("Reversed segment endpoints");
        payload = { blocks: document.blocks.slice(start, end + 1).flatMap(block => block.type === "paragraph"
          ? [{ id: block.attrs.id, text: format(block.text) }] : []) };
        revision = document.revision;
      } else if (label) {
        if (segment?.first != null || segment?.last != null) throw new Error("A label has no block anchors");
        const text = selected(label, context);
        payload = { text: format(text.text) };
        revision = text.revision;
      } else { return { status: "unavailable" }; }
      return { status: "ok", revision, payload };
    } catch (error) {
      return { status: "incompatible", reason: error instanceof Error ? error.message : "Invalid mock content" };
    }
  }

  /** Original-language outline only; neither translated text nor option labels enter it. */
  outline(): ContentOutline {
    const units: [string, ContentOutline["units"][string]][] = [];
    for (const [key, entry] of Object.entries(this.data.occurrences)) {
      if (this.withdrawn.has(key)) continue;
      const document = entry.realizations[entry.original];
      if (!document) throw new Error("Mock occurrence has no original realization");
      units.push([key, { blocks: document.blocks.map(block => block.attrs.id), markers: Object.fromEntries(document.blocks
        .flatMap(block => block.type === "narrata-choice" ? [[block.attrs.id, block.attrs.choice_point]] : [])) }]);
    }
    return { format_version: 1, provider: "rezics", units: Object.fromEntries(units) };
  }
}

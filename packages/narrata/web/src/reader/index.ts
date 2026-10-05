import { collectScreenContent, contentItemKey, resolveContent } from "../content/index.js";
import type { ContentResolver, ResolveContext, ResolveItem, Resolution } from "../content/index.js";
import type { BookView, CommitId, OptionId } from "../generated/book-view.js";
import type { Book } from "../runtime.js";
import { StoreSuperseded } from "../storage.js";

/** Also permits a host's worker proxy. The host retains ownership of the book and storage. */
export type ReaderBook = Pick<Book, "inspect" | "choose" | "checkout" | "reload" | "nextContentUnits">;
/** Provider diagnostics never reach a rendering slot. */
export type ReaderResolution = Exclude<Resolution, { status: "incompatible" }> | { readonly status: "incompatible" };
export interface ReaderContent {
  readonly item: ResolveItem;
  readonly resolution: ReaderResolution;
}
export interface ReaderScreen {
  readonly book: BookView;
  readonly context: ResolveContext;
  readonly content: readonly ReaderContent[];
}
export type ReaderSave =
  | { readonly status: "memory" }
  | { readonly status: "saved"; readonly at: string | null }
  | { readonly status: "saving" | "failed" | "superseded" | "reloading" };
export type SelectionIssue = "count" | "disabled" | "unknown" | "duplicate";
export type ReaderError =
  | { readonly kind: "content" | "runtime" | "reload" }
  | { readonly kind: "selection"; readonly issue: SelectionIssue };
export interface ReaderSnapshot {
  readonly phase: "idle" | "loading" | "ready" | "error";
  readonly screen: ReaderScreen | null;
  readonly selected: readonly OptionId[];
  readonly save: ReaderSave;
  readonly error: ReaderError | null;
}
export interface ReaderOptions {
  readonly context?: ResolveContext;
  readonly maxItems?: number;
  /** Book does not expose its storage connection; the host declares whether writes are durable. */
  readonly persistence?: "memory" | "durable";
  readonly savedAt?: string | null;
}
export interface ReaderSelection {
  readonly min: number;
  readonly max: number;
  readonly canSubmit: boolean;
  readonly issue: SelectionIssue | null;
}

function freeze<T>(value: T): T {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
  return value;
}

/** A cached external-store snapshot. All nested values are cloned at the boundary and frozen. */
export class ReaderController {
  private snapshot: ReaderSnapshot;
  private readonly listeners = new Set<() => void>();
  private context: ResolveContext;
  private readonly maxItems: number;
  private readonly durable: boolean;
  private pending: Promise<boolean> | undefined;
  private disposed = false;
  private prefetchTimer: ReturnType<typeof setTimeout> | undefined;
  private generation = 0;

  constructor(private readonly book: ReaderBook, private readonly resolver: ContentResolver, options: ReaderOptions = {}) {
    this.maxItems = options.maxItems ?? 4096;
    if (!Number.isSafeInteger(this.maxItems) || this.maxItems < 1) throw new RangeError("maxItems must be a positive safe integer");
    this.context = structuredClone(options.context ?? {});
    this.durable = options.persistence === "durable";
    this.snapshot = freeze({ phase: "idle", screen: null, selected: [], error: null,
      save: this.durable ? { status: "saved", at: options.savedAt ?? null } : { status: "memory" } });
  }

  readonly getSnapshot = (): ReaderSnapshot => this.snapshot;
  readonly subscribe = (listener: () => void): (() => void) => {
    if (this.disposed) return () => undefined;
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  /** Idempotent, including React Strict Mode's repeated mount effect. */
  start(): Promise<boolean> {
    if (this.pending) return this.pending;
    return this.snapshot.phase === "idle" ? this.refresh() : Promise.resolve(this.snapshot.phase === "ready");
  }

  /** Reinspect after a host import or restore. Failed writes require reload first. */
  refresh(): Promise<boolean> {
    if (!this.available() || this.needsReload()) return Promise.resolve(false);
    return this.run(false);
  }

  setContext(context: ResolveContext): Promise<boolean> {
    if (!this.available() || this.needsReload()) return Promise.resolve(false);
    this.context = structuredClone(context);
    return this.run(false);
  }

  selection(options: readonly OptionId[] = this.snapshot.selected): ReaderSelection | null {
    const interaction = this.snapshot.screen?.book.view.interaction;
    if (interaction?.kind !== "choose") return null;
    const unique = new Set(options);
    const issue = unique.size !== options.length ? "duplicate"
      : options.some(id => !interaction.options.some(option => option.id === id)) ? "unknown"
      : interaction.options.some(option => unique.has(option.id) && !option.enabled) ? "disabled"
      : options.length < interaction.min || options.length > interaction.max ? "count" : null;
    return { min: interaction.min, max: interaction.max, issue,
      canSubmit: this.available() && this.snapshot.phase === "ready" && !this.needsReload() && issue === null };
  }

  toggle(id: OptionId): boolean {
    const interaction = this.snapshot.screen?.book.view.interaction;
    if (!this.available() || this.needsReload() || this.snapshot.phase !== "ready" || interaction?.kind !== "choose") return false;
    const option = interaction.options.find(option => option.id === id);
    if (!option?.enabled) return false;
    const selected = new Set(this.snapshot.selected);
    if (selected.has(id)) selected.delete(id);
    else {
      if (selected.size >= interaction.max) return false;
      selected.add(id);
    }
    this.publish({ selected: interaction.options.filter(option => selected.has(option.id)).map(option => option.id), error: null });
    return true;
  }

  choose(options: readonly OptionId[] = this.snapshot.selected): Promise<boolean> {
    if (!this.available() || this.needsReload() || this.snapshot.phase !== "ready") return Promise.resolve(false);
    const selection = this.selection(options);
    const view = this.snapshot.screen?.book.view;
    if (!selection || view?.interaction.kind !== "choose") return Promise.resolve(false);
    if (selection.issue) {
      this.publish({ error: { kind: "selection", issue: selection.issue } });
      return Promise.resolve(false);
    }
    // Author order is deterministic even when a host supplies selections in click order.
    const ids = new Set(options);
    const ordered = view.interaction.options.filter(option => ids.has(option.id)).map(option => option.id);
    const point = view.interaction.choice_point;
    return this.run(true, () => this.book.choose(view.cursor, point, ordered));
  }

  checkout(commit: CommitId): Promise<boolean> {
    if (!this.available() || this.needsReload() || this.snapshot.phase !== "ready") return Promise.resolve(false);
    if (!this.snapshot.screen?.book.view.history.some(entry => entry.id === commit)) return Promise.resolve(false);
    return this.run(true, () => this.book.checkout(commit));
  }

  /** Discard unconfirmed changes, reopen durable state, then resolve that screen. */
  reload(): Promise<boolean> {
    if (!this.available()) return Promise.resolve(false);
    return this.run(false, () => this.book.reload(), true);
  }

  retry(): Promise<boolean> { return this.needsReload() ? this.reload() : this.refresh(); }

  /** Unsubscribe and cancel speculative work. This does not close a host-owned book. */
  dispose(): void {
    this.disposed = true;
    this.generation++;
    clearTimeout(this.prefetchTimer);
    this.listeners.clear();
  }

  private available(): boolean { return !this.disposed && !this.pending; }
  private needsReload(): boolean { return ["failed", "superseded", "reloading"].includes(this.snapshot.save.status); }
  private publish(update: Partial<ReaderSnapshot>): void {
    if (this.disposed) return;
    this.snapshot = freeze({ ...this.snapshot, ...update });
    for (const listener of this.listeners) listener();
  }

  private run(mutation: boolean, action?: () => Promise<unknown>, reload = false): Promise<boolean> {
    const generation = ++this.generation;
    clearTimeout(this.prefetchTimer);
    // Start on a microtask so pending is set before subscribers can invoke another action.
    const pending = Promise.resolve().then(async () => {
      this.publish({ phase: "loading", error: null,
        ...(reload ? { save: { status: "reloading" } } : mutation && this.durable ? { save: { status: "saving" } } : {}) });
      let stage: "runtime" | "content" | "reload" = reload ? "reload" : "runtime";
      let confirmed = false;
      try {
        await action?.();
        confirmed = true;
        if (mutation || reload) this.publish({ save: this.durable ? { status: "saved", at: new Date().toISOString() } : { status: "memory" } });
        const book = structuredClone(await this.book.inspect());
        stage = "content";
        const context = structuredClone(this.context);
        const items = collectScreenContent(book);
        const results = await resolveContent(this.resolver, items, context, this.maxItems);
        const content = items.map((item, index): ReaderContent => {
          const result = results[index];
          if (!result) throw new Error("Content resolver omitted a result");
          return { item, resolution: result.status === "incompatible" ? { status: "incompatible" } : structuredClone(result) };
        });
        if (this.disposed) return false;
        if (this.pending === pending) this.pending = undefined;
        this.publish({ phase: "ready", screen: { book, context, content }, selected: [], error: null });
        // Speculation starts after publishing the first screen; it is never on its critical path.
        this.prefetchTimer = setTimeout(() => { void this.prefetch(generation, context); }, 0);
        return true;
      } catch (error) {
        if (this.pending === pending) this.pending = undefined;
        this.publish({ phase: "error", error: { kind: stage },
          ...(!confirmed && mutation ? { save: { status: superseded(error) ? "superseded" : "failed" } } : {}) });
        return false;
      }
    }).finally(() => { if (this.pending === pending) this.pending = undefined; });
    this.pending = pending;
    return pending;
  }

  private async prefetch(generation: number, context: ResolveContext): Promise<void> {
    try {
      if (this.disposed || generation !== this.generation) return;
      const units = await this.book.nextContentUnits();
      if (this.disposed || generation !== this.generation) return;
      // Whole body units only; future labels, arguments and topology never reach the host.
      await resolveContent(this.resolver, units.map(unit => ({ content: { unit } })), context, this.maxItems);
    } catch { /* Speculation cannot turn a confirmed screen into a reader error. */ }
  }
}

function superseded(error: unknown): boolean {
  // Book's Wasm boundary still reports stale expected cursors as the native error text.
  return error instanceof StoreSuperseded || (error instanceof Error && /^stale_input(?:\s|:)/.test(error.message));
}

export function createReader(book: ReaderBook, resolver: ContentResolver, options?: ReaderOptions): ReaderController {
  return new ReaderController(book, resolver, options);
}

const contentIndexes = new WeakMap<ReaderScreen, ReadonlyMap<string, ReaderResolution>>();
export function readerContent(screen: ReaderScreen, item: ResolveItem): ReaderResolution | undefined {
  let index = contentIndexes.get(screen);
  if (!index) {
    index = new Map(screen.content.map(entry => [contentItemKey(entry.item), entry.resolution]));
    contentIndexes.set(screen, index);
  }
  return index.get(contentItemKey(item));
}

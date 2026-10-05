import { useEffect, useId, useRef, useSyncExternalStore } from "react";
import type { ReactNode } from "react";
import { readerContent } from "../reader/index.js";
import type { ReaderController, ReaderResolution, ReaderScreen, ReaderSnapshot } from "../reader/index.js";
import type { Content, ResolveContext, ResolveItem } from "../content/index.js";
import type { PresentationItem, VariableView, ViewScalar } from "../generated/book-view.js";

export type ContentRole = "body" | "reply" | "title" | "product-title" | "option" | "disabled-reason" | "history-title" | "variable-label" | "variable-value" | "ending-body";
export interface ContentSlotProps {
  readonly content: Content;
  readonly item: ResolveItem;
  readonly resolution: ReaderResolution;
  readonly context: ResolveContext;
  readonly role: ContentRole;
  readonly presentation?: { readonly execution: string; readonly commit: string; readonly node: string; readonly occurrence: number };
}
export type ContentSlot = (props: ContentSlotProps) => ReactNode;
export interface ReaderSlots {
  readonly body: ContentSlot;
  readonly option: ContentSlot;
  readonly reference: ContentSlot;
}
export interface ReaderMessages {
  readonly loading: string;
  readonly unavailable: string;
  readonly incompatible: string;
  readonly error: string;
  readonly retry: string;
  readonly reload: string;
  readonly memory: string;
  readonly saving: string;
  readonly saved: string;
  readonly failed: string;
  readonly superseded: string;
  readonly reloading: string;
  readonly choices: string;
  readonly path: string;
  readonly state: string;
  readonly previous: string;
  readonly next: string;
  readonly finished: string;
  readonly selection: (min: number, max: number) => string;
  readonly submit: (count: number) => string;
}
export const readerMessages: ReaderMessages = {
  loading: "Opening story…", unavailable: "Content unavailable", incompatible: "Content cannot be displayed",
  error: "Unable to update the story.", retry: "Retry", reload: "Reload saved progress", memory: "Progress is kept in this page",
  saving: "Saving…", saved: "Saved", failed: "Progress could not be saved. Reload before continuing.",
  superseded: "Another page updated this journey. Reload before continuing.", reloading: "Reloading saved progress…",
  choices: "Available actions", path: "Journey", state: "Story state", previous: "Previous step", next: "Next step", finished: "Story finished",
  selection: (min, max) => min === max ? `Choose ${min}` : `Choose ${min}–${max}`,
  submit: count => count === 0 ? "Continue without selecting" : `Confirm (${count})`,
};
export type ReaderClasses = Partial<Record<"reader" | "heading" | "productTitle" | "prose" | "lead" | "reply" | "subtitle" | "choices" | "multi" | "choiceItem" | "choice" | "choiceCheck" | "reason" | "submit" | "footer" | "previous" | "next" | "save" | "path" | "state" | "error", string>>;
export interface ReaderProps {
  readonly controller: ReaderController;
  readonly slots: ReaderSlots;
  readonly messages?: ReaderMessages;
  readonly classes?: ReaderClasses;
  readonly headingId?: string;
  readonly showPath?: boolean;
  readonly showState?: boolean;
  readonly showNavigation?: boolean;
  /** Pause interactive controls while the host imports a work or performs another operation. */
  readonly disabled?: boolean;
  readonly children?: ReactNode;
}

export function useReader(controller: ReaderController): ReaderSnapshot {
  const snapshot = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useEffect(() => { void controller.start(); }, [controller]);
  return snapshot;
}

function renderReference(screen: ReaderScreen, slots: ReaderSlots, item: ResolveItem, role: ContentRole, presentation?: PresentationItem): ReactNode {
  const resolution = readerContent(screen, item) ?? { status: "unavailable" };
  const props: ContentSlotProps = { content: item.content, item, resolution, context: screen.context, role,
    ...(presentation ? { presentation: { execution: screen.book.view.execution, commit: presentation.commit, node: presentation.node, occurrence: presentation.occurrence } } : {}) };
  return role === "option" ? slots.option(props) : ["body", "reply", "ending-body"].includes(role) ? slots.body(props) : slots.reference(props);
}

function itemContent(screen: ReaderScreen, slots: ReaderSlots, item: PresentationItem): ReactNode {
  return renderReference(screen, slots, { content: item.content, args: item.args }, item.role, item);
}
function itemKey(item: PresentationItem): string { return `${item.commit}:${item.occurrence}`; }
function className(classes: ReaderClasses, name: keyof ReaderClasses): string { return classes[name] ?? `narrata-reader__${name}`; }

export function ReaderSaveStatus({ controller, messages = readerMessages, className: css, disabled = false }: { readonly controller: ReaderController; readonly messages?: ReaderMessages; readonly className?: string; readonly disabled?: boolean }) {
  const snapshot = useReader(controller);
  const save = snapshot.save;
  const failed = save.status === "failed" || save.status === "superseded";
  return <div className={css ?? "narrata-reader__save"}>
    <p role={failed ? "alert" : "status"}>{messages[save.status]}{save.status === "saved" && save.at ? <> <time dateTime={save.at}>{new Date(save.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time></> : null}</p>
    {failed ? <button disabled={disabled || snapshot.phase === "loading"} onClick={() => { void controller.reload(); }}>{messages.reload}</button> : null}
  </div>;
}

export function ReaderPath({ controller, slots, messages = readerMessages, className: css, disabled = false }: { readonly controller: ReaderController; readonly slots: ReaderSlots; readonly messages?: ReaderMessages; readonly className?: string; readonly disabled?: boolean }) {
  const snapshot = useReader(controller);
  const screen = snapshot.screen;
  if (!screen) return null;
  const branches = new Map<string, number>();
  for (const entry of screen.book.view.history) if (entry.parent) branches.set(entry.parent, (branches.get(entry.parent) ?? 0) + 1);
  return <nav aria-label={messages.path}><ol className={css ?? "narrata-reader__path"}>
    {screen.book.view.history.map(entry => <li key={entry.id} className={[entry.current ? "current" : "", (branches.get(entry.parent ?? "") ?? 0) > 1 ? "branch" : ""].join(" ")}>
      <button disabled={disabled || snapshot.phase !== "ready"} aria-current={entry.current ? "step" : undefined} onClick={() => { void controller.checkout(entry.id); }}>
        {entry.title ? renderReference(screen, slots, { content: entry.title }, "history-title") : entry.key ?? `${messages.path} ${entry.depth}`}
      </button>
    </li>)}
  </ol></nav>;
}

export function ReaderNavigation({ controller, messages = readerMessages, classes = {}, disabled = false }: { readonly controller: ReaderController; readonly messages?: ReaderMessages; readonly classes?: ReaderClasses; readonly disabled?: boolean }) {
  const snapshot = useReader(controller);
  const view = snapshot.screen?.book.view;
  const here = view?.history.find(entry => entry.current);
  const children = view?.history.filter(entry => entry.parent === view.cursor) ?? [];
  const interaction = view?.interaction;
  const only = interaction?.kind === "choose" && interaction.min === 1 && interaction.max === 1 && interaction.options.length === 1 ? interaction.options[0] : undefined;
  const next = only?.enabled ? only : undefined;
  return <footer className={className(classes, "footer")}>
    <button className={className(classes, "previous")} disabled={disabled || snapshot.phase !== "ready" || !here?.parent} onClick={() => { if (here?.parent) void controller.checkout(here.parent); }}>{messages.previous}</button>
    <ReaderSaveStatus controller={controller} messages={messages} className={className(classes, "save")} disabled={disabled} />
    <button className={className(classes, "next")} disabled={disabled || snapshot.phase !== "ready" || (!next && children.length !== 1)} onClick={() => {
      if (next) void controller.choose([next.id]); else if (children[0]) void controller.checkout(children[0].id);
    }}>{messages.next}</button>
  </footer>;
}

function scalar(value: ViewScalar, screen: ReaderScreen, slots: ReaderSlots): ReactNode {
  return value.type === "ref" ? renderReference(screen, slots, { content: value.value }, "variable-value") : String(value.value);
}

export function Reader({ controller, slots, messages = readerMessages, classes = {}, headingId, showPath = true, showState = true, showNavigation = true, disabled = false, children }: ReaderProps) {
  const snapshot = useReader(controller);
  const unique = useId();
  const titleId = headingId ?? `${unique}-title`;
  const heading = useRef<HTMLHeadingElement>(null);
  const previous = useRef<string | undefined>(undefined);
  const cursor = snapshot.screen?.book.view.cursor;
  useEffect(() => {
    if (cursor && previous.current && previous.current !== cursor) heading.current?.focus({ preventScroll: true });
    previous.current = cursor;
  }, [cursor]);
  const screen = snapshot.screen;
  if (!screen) return <div className={className(classes, "reader")}>
    <p role={snapshot.error ? "alert" : "status"}>{snapshot.error ? messages.error : messages.loading}</p>
    {snapshot.error ? <button disabled={disabled} onClick={() => { void controller.retry(); }}>{messages.retry}</button> : null}
  </div>;
  const { book } = screen;
  const { view } = book;
  const interaction = view.interaction;
  let titleIndex = -1;
  book.page.forEach((item, index) => { if (item.role === "title") titleIndex = index; });
  if (interaction.kind === "finished") titleIndex = book.page.length;
  const title = book.page[titleIndex];
  const selection = controller.selection();
  const busy = disabled || snapshot.phase !== "ready";
  const single = interaction.kind === "choose" && interaction.min === 1 && interaction.max === 1;
  const variables = (values: readonly VariableView[]) => values.map(variable => <div key={variable.name}>
    <dt>{variable.label ? renderReference(screen, slots, { content: variable.label }, "variable-label") : variable.name}</dt><dd>{scalar(variable.value, screen, slots)}</dd>
  </div>);
  const present = (item: PresentationItem) => item.role === "title"
    ? <h2 key={itemKey(item)} className={className(classes, "subtitle")}>{itemContent(screen, slots, item)}</h2>
    : item.role === "reply" ? <blockquote key={itemKey(item)} className={className(classes, "reply")}>{itemContent(screen, slots, item)}</blockquote>
      : <div key={itemKey(item)}>{itemContent(screen, slots, item)}</div>;
  return <div>
    <article className={className(classes, "reader")} aria-labelledby={titleId} aria-busy={disabled || snapshot.phase === "loading"}>
      {titleIndex > 0 ? <div className={`${className(classes, "prose")} ${className(classes, "lead")}`}>{book.page.slice(0, titleIndex).map(present)}</div> : null}
      <div className={className(classes, "heading")}>
        {view.product.title ? <p className={className(classes, "productTitle")}>{renderReference(screen, slots, { content: view.product.title }, "product-title")}</p> : null}
        <h1 ref={heading} id={titleId} tabIndex={-1}>{interaction.kind === "finished" ? interaction.title ? renderReference(screen, slots, { content: interaction.title }, "title") : messages.finished
          : title ? itemContent(screen, slots, title) : view.history.find(entry => entry.current)?.key ?? view.product.id}</h1>
      </div>
      <div className={className(classes, "prose")}>
        {book.page.slice(titleIndex + 1).map(present)}
        {interaction.kind === "finished" && interaction.body ? renderReference(screen, slots, { content: interaction.body }, "ending-body") : null}
      </div>
      {interaction.kind === "choose" ? <fieldset className={[className(classes, "choices"), single ? "" : className(classes, "multi")].join(" ")} aria-describedby={`${unique}-hint`}>
        <legend id={`${unique}-hint`}>{single ? messages.choices : messages.selection(interaction.min, interaction.max)}</legend>
        {interaction.options.map((option, index) => {
          const checked = snapshot.selected.includes(option.id);
          const reasonId = `${unique}-reason-${index}`;
          const description = !option.enabled && option.reason ? reasonId : undefined;
          const label = option.label ? renderReference(screen, slots, { content: option.label, args: interaction.args }, "option") : option.key ?? `${messages.choices} ${index + 1}`;
          return <div key={option.id} className={className(classes, "choiceItem")}>
            {single ? <button className={className(classes, "choice")} disabled={busy || !option.enabled} aria-describedby={description} onClick={() => { void controller.choose([option.id]); }}>{label}</button>
              : <label className={`${className(classes, "choice")} ${className(classes, "choiceCheck")}`}>
                <input type="checkbox" checked={checked} disabled={busy || !option.enabled || (!checked && snapshot.selected.length >= interaction.max)} aria-describedby={description} onChange={() => { controller.toggle(option.id); }} />{label}
              </label>}
            {description && option.reason ? <p className={className(classes, "reason")} id={reasonId}>{renderReference(screen, slots, { content: option.reason, args: interaction.args }, "disabled-reason")}</p> : null}
          </div>;
        })}
        {!single ? <button className={className(classes, "submit")} disabled={disabled || !selection?.canSubmit} onClick={() => { void controller.choose(); }}>{messages.submit(snapshot.selected.length)}</button> : null}
      </fieldset> : null}
      {snapshot.error && !["failed", "superseded"].includes(snapshot.save.status) ? <div className={className(classes, "error")} role="alert">{messages.error} <button disabled={disabled} onClick={() => { void controller.retry(); }}>{messages.retry}</button></div> : null}
      {showState ? <section aria-label={messages.state} className={className(classes, "state")}>
        <dl>{variables(view.shared)}</dl>
        {view.frames.filter(frame => frame.parameters.length + frame.locals.length > 0).map(frame => <dl key={frame.instance} aria-label={`${frame.graph.package}.${frame.graph.graph} · ${frame.instance}`}>
          {variables([...frame.parameters, ...frame.locals])}
        </dl>)}
      </section> : null}
      {children}
    </article>
    {showNavigation ? <ReaderNavigation controller={controller} messages={messages} classes={classes} disabled={disabled} /> : null}
    {showPath ? <ReaderPath controller={controller} slots={slots} messages={messages} className={className(classes, "path")} disabled={disabled} /> : null}
  </div>;
}

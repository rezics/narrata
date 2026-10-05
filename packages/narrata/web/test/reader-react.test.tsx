import { expect, test } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { createReader, openBook } from "@rezics/narrata";
import { Reader, type ContentSlotProps, type ReaderSlots } from "@rezics/narrata/react";
import { demoPack, execution } from "./fixture.js";

test("host slots receive references, sanitized resolutions, context and stable presentation keys", async () => {
  const book = await openBook({ pack: demoPack, execution });
  const received: ContentSlotProps[] = [];
  const slot = (props: ContentSlotProps) => {
    received.push(props);
    return <span data-host-role={props.role}>{props.resolution.status === "ok" && "text" in props.resolution.payload ? props.resolution.payload.text : props.resolution.status}</span>;
  };
  const slots: ReaderSlots = { body: slot, option: slot, reference: slot };
  const controller = createReader(book, { resolve: async request => request.items.map(item => "unit" in item.content
    ? { status: "incompatible", reason: "private diagnostics" }
    : { status: "ok", revision: "host-revision", payload: { text: "Host text" } }) }, { context: { languages: ["en"], realization: "edition" } });
  try {
    await controller.start();
    const html = renderToStaticMarkup(<Reader controller={controller} slots={slots} />);
    expect(html).toContain('aria-label="Journey"');
    expect(html).toContain('aria-current="step"');
    expect(html).toContain('data-host-role="option"');
    expect(html).not.toContain("private diagnostics");
    const body = received.find(props => props.role === "body");
    expect(body?.content).toHaveProperty("unit");
    expect(body?.resolution).toEqual({ status: "incompatible" });
    expect(body?.context).toEqual({ languages: ["en"], realization: "edition" });
    expect(body?.presentation).toMatchObject({ execution, occurrence: expect.any(Number), commit: expect.any(String) });
    expect(received.some(props => props.role === "history-title")).toBe(true);
    expect(received.some(props => props.role === "product-title")).toBe(true);
    const pausedSnapshot = controller.getSnapshot();
    const paused = renderToStaticMarkup(<Reader controller={controller} slots={slots} disabled />);
    expect(paused).toContain('aria-busy="true"');
    const buttons = [...paused.matchAll(/<button\b[^>]*>/g)].map(match => match[0]);
    expect(buttons.length).toBeGreaterThan(0);
    expect(buttons.every(button => button.includes(" disabled"))).toBe(true);
    expect(controller.getSnapshot()).toBe(pausedSnapshot);
    const view = controller.getSnapshot().screen?.book.view;
    if (view?.interaction.kind !== "choose") throw new Error("Expected an interaction");
    const camp = view.interaction.options.find(option => option.key === "camp");
    if (!camp) throw new Error("Expected the called graph");
    expect(await controller.choose([camp.id])).toBe(true);
    received.length = 0;
    const called = renderToStaticMarkup(<Reader controller={controller} slots={slots} />);
    expect(called).toContain('data-host-role="variable-value"');
    const value = received.find(props => props.role === "variable-value");
    expect(value?.content).toEqual({ provider: "local", key: "main:visitor" });
    expect(value?.context).toEqual({ languages: ["en"], realization: "edition" });
  } finally { controller.dispose(); await book.close(); }
});

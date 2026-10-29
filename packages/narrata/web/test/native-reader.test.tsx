import { expect, test } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { createReader, openBook, type ReaderBook, type ResolveRequest } from "@rezics/narrata";
import { Reader, type ContentSlotProps, type ReaderSlots } from "@rezics/narrata/react";
import { createNativeContentAdapter } from "../src/content/native.js";
import { nativeDocumentSample, type SampleNativeValue } from "../src/testing/native-document.js";
import { demoPack, execution } from "./fixture.js";

test("Reader slots render native marks and media from their resolved revision and recover after unavailable content", async () => {
  const real = await openBook({ pack: demoPack, execution });
  const view = await real.inspect();
  await real.close();
  const first = view.page.find(item => item.role === "body");
  if (!first) throw new Error("No body presentation");
  first.content = { unit: { provider: "native", key: "chapter" }, first: "arrival", last: "map" };
  const book: ReaderBook = { inspect: async () => view, choose: async () => view.view.cursor, checkout: async () => undefined,
    reload: async () => undefined, nextContentUnits: async () => [] };
  const requests: ResolveRequest[] = [];
  let unavailable = true, badAnchors = false, revision = "A";
  const adapter = createNativeContentAdapter<SampleNativeValue>({ resolve: async request => {
    requests.push(structuredClone(request));
    return request.items.map(item => {
      if (!("unit" in item.content)) return { status: "ok", revision: "labels:A", payload: { text: `Label in ${request.context.realization}` } };
      if (unavailable) return { status: "unavailable" };
      const snapshot = nativeDocumentSample(revision);
      return { status: "document", document: badAnchors ? { ...snapshot, blocks: snapshot.blocks.slice(1) } : snapshot };
    });
  } });
  const received: ContentSlotProps[] = [];
  const slot = (props: ContentSlotProps) => {
    received.push(props);
    if (props.resolution.status !== "ok") return <span>{props.resolution.status}</span>;
    const native = adapter.lookup(props.item, props.context, props.resolution);
    if (!native) return "text" in props.resolution.payload ? props.resolution.payload.text : props.resolution.payload.blocks.map(block => block.text).join(" ");
    return <div data-native-revision={native.revision}>{native.blocks.map(block => block.value.kind === "image"
      ? <span key={block.id} role="img" aria-label={block.value.alt} data-host-media={block.value.source} />
      : <p key={block.id}>{block.value.runs.map((run, index) => run.strong ? <strong key={index}>{run.text}</strong> : run.text)}</p>)}</div>;
  };
  const slots: ReaderSlots = { body: slot, option: slot, reference: slot };
  const context = { languages: ["en"], realization: "selected-edition", viewer: "reader:A" };
  const reader = createReader(book, adapter, { context, maxItems: 2 });
  try {
    expect(await reader.start()).toBe(true);
    expect(renderToStaticMarkup(<Reader controller={reader} slots={slots} />)).toContain("unavailable");
    unavailable = false;
    expect(await reader.refresh()).toBe(true);
    revision = "B"; // Rendering A does not fetch the host's new current revision.
    const html = renderToStaticMarkup(<Reader controller={reader} slots={slots} />);
    expect(html).toContain('<strong>mountain map</strong>');
    expect(html).toContain('data-native-revision="A"');
    expect(html).toContain('data-host-media="host-media:mountain-map"');
    expect(html).toContain("Label in selected-edition");
    expect(JSON.stringify(reader.getSnapshot())).not.toContain('"runs"');
    expect(requests.every(request => JSON.stringify(request.context) === JSON.stringify(context))).toBe(true);
    expect(requests.every(request => request.items.length <= 2)).toBe(true);
    expect(received.some(props => props.role === "option" && props.context.realization === context.realization)).toBe(true);
    badAnchors = true;
    expect(await reader.refresh()).toBe(true);
    received.length = 0;
    const incompatible = renderToStaticMarkup(<Reader controller={reader} slots={slots} />);
    expect(incompatible).toContain("incompatible");
    expect(incompatible).not.toContain("Missing native segment endpoint");
    expect(received.find(props => props.role === "body")?.resolution).toEqual({ status: "incompatible" });
    badAnchors = false;
    expect(await reader.refresh()).toBe(true);
    expect(renderToStaticMarkup(<Reader controller={reader} slots={slots} />)).toContain('data-native-revision="B"');
    adapter.clear();
    const fallback = renderToStaticMarkup(<Reader controller={reader} slots={slots} />);
    expect(fallback).toContain("A letter waits beside the mountain map.");
    expect(fallback).not.toContain("data-native-revision");
  } finally { reader.dispose(); adapter.clear(); }
});

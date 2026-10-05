import { createReader, type Book, type ContentResolver } from "@rezics/narrata";
import { Reader, ReaderPath, ReaderNavigation, ReaderSaveStatus, useReader, type ReaderSlots } from "@rezics/narrata/react";

function readerTypes(book: Book, resolver: ContentResolver) {
  const controller = createReader(book, resolver, { context: { languages: ["en"] } });
  const slots: ReaderSlots = {
    body: props => props.resolution.status === "ok" ? <div data-unit={"unit" in props.content ? props.content.unit.key : props.content.key} /> : props.resolution.status,
    option: props => props.context.realization ?? props.content.toString(),
    reference: props => props.role,
  };
  const snapshot = useReader(controller);
  const item = snapshot.screen?.content[0];
  if (item?.resolution.status === "incompatible") {
    // @ts-expect-error Diagnostic reasons are absent from the rendering contract.
    void item.resolution.reason;
  }
  // @ts-expect-error Published snapshots cannot be assigned to.
  snapshot.phase = "ready";
  return <><Reader controller={controller} slots={slots} /><ReaderPath controller={controller} slots={slots} />
    <ReaderNavigation controller={controller} /><ReaderSaveStatus controller={controller} /></>;
}
void readerTypes;

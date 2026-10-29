// The reader subpath: the slim runtime Wasm, content resolution and the reader controller.
// Authoring, publication, layout and the legacy raw module's publishGraph never load from here.
export { openBook, initialize, LocalContent, checkedBook, decodeBook } from "../runtime.js";
export type { Book, OpenBookOptions, WasmSource } from "../runtime.js";
export type * from "../generated/book-view.js";
export { collectScreenContent, contentItemKey, contentRequests, resolveContent } from "../content/index.js";
export type { ContentResolver, Content, ResolveContext, ResolveItem, ResolveRequest, Resolution, Payload, TextBlock, ContentOutline, UnitOutline } from "../content/index.js";
export { createReader, ReaderController, readerContent } from "./controller.js";
export type { ReaderBook, ReaderOptions, ReaderSnapshot, ReaderScreen, ReaderContent, ReaderResolution, ReaderSave, ReaderError, ReaderSelection, SelectionIssue } from "./controller.js";

import { expect, test, vi } from "vitest";
import { AuthenticatedLibrary, LibraryClosed, WriteRejected } from "../src/online";
import { encodeLoaded, encodeLoadRequest, type StoreState } from "../src/protocol";

const request = encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] });
const state: StoreState = { id: new Uint8Array(16).fill(1), revision: 7 };
const loaded = (store = state) => encodeLoaded({ store, keys: [], objects: [], keyRanges: [], objectRanges: [] });

test("closing rejects immediately, drains late replies, clears content and releases once", async () => {
  let resolve!: (value: Uint8Array) => void;
  const load = vi.fn(() => new Promise<Uint8Array>(done => { resolve = done; }));
  const clearContent = vi.fn(); const release = vi.fn();
  const library = new AuthenticatedLibrary({ storeId: state.id, store: { load, persist: load }, release, clearContent, remove: vi.fn() });
  const pending = library.load(request);
  await Promise.resolve();
  const close = library.close();
  expect(clearContent).toHaveBeenCalledTimes(1);
  await expect(library.load(request)).rejects.toBeInstanceOf(LibraryClosed);
  expect(release).not.toHaveBeenCalled();
  resolve(loaded());
  await expect(pending).rejects.toBeInstanceOf(LibraryClosed);
  await close; await library.close();
  expect(release).toHaveBeenCalledTimes(1);
  expect(library.observed).toBeUndefined();
});

test("deletion captures checked observed identity/revision and old handles cannot reopen a replacement", async () => {
  const remove = vi.fn(async (_observed: StoreState) => undefined);
  const load = vi.fn(async () => loaded());
  const id = state.id.slice();
  const library = new AuthenticatedLibrary({ storeId: id, store: { load, persist: load }, release: () => undefined, clearContent: () => undefined, remove });
  id.fill(9);
  await library.load(request);
  library.observed?.id.fill(0);
  await library.remove();
  expect(remove).toHaveBeenCalledExactlyOnceWith(state);
  await expect(library.load(request)).rejects.toBeInstanceOf(LibraryClosed);
  expect(load).toHaveBeenCalledTimes(1);
  const replacement = new AuthenticatedLibrary({ storeId: state.id, store: { load: async () => loaded({ id: new Uint8Array(16).fill(2), revision: 0 }), persist: load }, release: () => undefined, clearContent: () => undefined, remove });
  await expect(replacement.load(request)).rejects.toBeInstanceOf(WriteRejected);
  await replacement.close();
});

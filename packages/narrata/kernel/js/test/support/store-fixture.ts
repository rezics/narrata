import type { CacheStore } from "../../src/host";
import { IndexedDbStore } from "../../src/indexeddb";

/** Adapter-independent lifecycle and transaction fault controls, used only by tests. */
export interface StoreFixture {
  store: CacheStore;
  /** A second handle to the same library, for competing writers. */
  connect(): Promise<CacheStore>;
  /** Close existing handles before opening the durable library again. */
  reopen(): Promise<CacheStore>;
  clear(): Promise<CacheStore>;
  /** Abort the next valid persist after staging this many key writes. */
  failNextPersist(afterWrites?: number): void;
  /** Observe the storage commit event itself, before a promise can report it. */
  observeNextCommit(committed: () => void): void;
  dispose(): Promise<void>;
}

export type StoreFactory = () => Promise<StoreFixture>;

/** Works with either fake-indexeddb or the browser's native implementation. */
export async function indexedDbFixture(name: string, factory: IDBFactory): Promise<StoreFixture> {
  let store = await IndexedDbStore.open(name, factory);
  const handles = [store];
  let restore: (() => void) | undefined;
  let restoreObserver: (() => void) | undefined;
  const connect = async () => {
    store = await IndexedDbStore.open(name, factory);
    handles.push(store);
    return store;
  };
  return {
    store,
    connect,
    async reopen() {
      for (const handle of handles) handle.close();
      return connect();
    },
    async clear() {
      for (const handle of handles) handle.close();
      await IndexedDbStore.delete(name, factory);
      return connect();
    },
    failNextPersist(afterWrites = 1) {
      const original = IDBObjectStore.prototype.put;
      restore = () => { IDBObjectStore.prototype.put = original; };
      IDBObjectStore.prototype.put = function (value: unknown, key?: IDBValidKey) {
        const result = original.call(this, value, key);
        if (this.name === "keys" && --afterWrites === 0) {
          restore?.();
          restore = undefined;
          throw new DOMException("injected quota failure after a staged write", "QuotaExceededError");
        }
        return result;
      };
    },
    observeNextCommit(committed) {
      const original = IDBDatabase.prototype.transaction;
      restoreObserver = () => { IDBDatabase.prototype.transaction = original; };
      IDBDatabase.prototype.transaction = function (names, mode, options) {
        const transaction = original.call(this, names, mode, options);
        if (mode === "readwrite") {
          transaction.addEventListener("complete", committed, { once: true });
          restoreObserver?.();
          restoreObserver = undefined;
        }
        return transaction;
      };
    },
    async dispose() {
      restore?.();
      restoreObserver?.();
      for (const handle of handles) handle.close();
      await IndexedDbStore.delete(name, factory);
    },
  };
}

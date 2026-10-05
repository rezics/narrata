// The page the browser tests drive: the counter module's cache over this package's IndexedDB
// adapter. `?db=` names the database, so each test starts from its own.
import { IndexedDbStore, StorageHost } from "../src/index";
import type { CounterPage, Outcome } from "./counter-page";
import init, { CounterSaves } from "./generated/narrata_storage_browser_counter.js";

const database = new URLSearchParams(location.search).get("db") ?? "narrata-storage";
await init();
let store = await IndexedDbStore.open(database);
let saves = new CounterSaves();
let host = new StorageHost(store, saves);

async function outcome<T>(operation: () => T): Promise<Outcome<T>> {
  try {
    return { ok: await host.run(operation) };
  } catch (error) {
    const failure = error instanceof Error ? error : new Error(String(error));
    return { error: failure.name, message: failure.message };
  }
}

const page: CounterPage = {
  async open() {
    await host.run(() => saves.reload());
    return outcome(() => saves.open());
  },
  advance: (increment) => outcome(() => saves.advance(increment)),
  save: () => outcome(() => saves.save()),
  reads: () => ({ ...store.reads }),
  export: async () => Array.from(await store.export()),
  async restore(exported) {
    store.close();
    await IndexedDbStore.delete(database);
    store = await IndexedDbStore.open(database);
    await store.import(new Uint8Array(exported));
    saves = new CounterSaves();
    host = new StorageHost(store, saves);
  },
  requestPersistence: () => IndexedDbStore.requestPersistence(),
};

Object.assign(globalThis, { counter: page });
document.querySelector("#status")!.textContent = "ready";

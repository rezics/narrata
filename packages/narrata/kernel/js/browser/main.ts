// The page the browser tests drive: the counter module's cache over this package's IndexedDB
// adapter. `?db=` names the database, so each test starts from its own.
import { IndexedDbStore, StorageHost, decodeLoaded, encodeLoadRequest, encodeFlush } from "../src/index";
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
  async largeColdOperations() {
    const db = await IndexedDbStore.open(`large-cold-${crypto.randomUUID()}`);
    const state = decodeLoaded(await db.load(encodeLoadRequest({keys:[],objects:[],keyRanges:[],objectRanges:[]}))).store;
    const keys = Array.from({length:1025}, (_,i): [Uint8Array,Uint8Array] => [new Uint8Array([0,9,i>>8,i&255]),new Uint8Array([1])]);
    await db.persist(encodeFlush({store:state.id,batches:[{base:state.revision,revision:state.revision+1,putKeys:keys,putObjects:[],deleteKeys:[],deleteObjects:[]}]}));
    const raw = new CounterSaves();
    const driver = new StorageHost(db,raw);
    try {
      let before = db.reads.loads;
      const scan = await driver.run(()=>raw.scanKeys());
      const scanLoads = db.reads.loads-before;
      raw.reload();before = db.reads.loads;
      const absent = await driver.run(()=>raw.getObjects());
      const absenceLoads = db.reads.loads-before;
      raw.reload();before = db.reads.loads;
      await driver.run(()=>raw.checkedBatch());
      const batchLoads=db.reads.loads-before;
      raw.reload();before=db.reads.loads;
      const points=await driver.run(()=>raw.getObjects());
      return {scan,points,absent,scanLoads,pointLoads:db.reads.loads-before,absenceLoads,batchLoads};
    } finally {raw.free();db.close();}
  },
  async aggregateFlushBytes() {
    const db=await IndexedDbStore.open(`aggregate-bytes-${crypto.randomUUID()}`),raw=new CounterSaves();
    let persists=0;
    const driver=new StorageHost({load:r=>db.load(r),persist:r=>{persists++;return db.persist(r);}},raw);
    try {
      await driver.run(()=>raw.open());persists=0;
      let rejected=false;
      try {await driver.run(()=>{for(let index=0;index<4;index++)raw.queueBytes(index,16*1024*1024);raw.queueBytes(4,1);});}
      catch(error) {if(!(error instanceof Error) || !error.message.includes("BatchBytes"))throw error;rejected=true;}
      const queuedBytes=raw.queuedBytes(),writes=persists;
      raw.discardCandidate();await driver.discardPendingAfterReload();
      const reopened=await driver.run(()=>raw.open());
      return {queuedBytes,persists:writes,reopened,rejected};
    } finally {raw.free();db.close();}
  },
  async open() {
    saves.reload();
    await host.discardPendingAfterReload();
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
